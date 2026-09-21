# Cowchat owner-stream integration

Chad's September 21 decisions, relayed by Claude in `dashboard-cowchat-v2`
at sequence 121 and confirmed by Chad in the implementation task:

- CBQS is the launch room-log authority.
- One stream per owner/tenant, rooms as lanes, one active writer per stream.
- Broker-host HA follows launch. Single-broker restart recovery is required.
- Connectors remain generic. Claude relayed Chad's order at sequence 136:
  Telegram, then Slack, then Linq. Linq waits for API credentials; it does not
  block the other work.

This branch starts from Cowchat `b00dab6`. The existing local server and its
data remain in place while the hosted path is built. No merge or deployment
is part of this integration step.

## First executable slice

Exercise actual Cowchat room creation, encrypted append and history over the
owner-stream log. Restart with an empty local projection and recover identical
room/message state by replay. Preserve message-ID retry identity and reject
changed content under an existing ID. The accepted result must follow durable
log commitment and required archive capture, not an earlier local write.

Expected modules:

- `room_log.rs` and its CBQS implementation: bounded append/replay/subscription
  operations, finite-epoch grant sessions, signed fence before takeover replay.
- A typed command/replay module: stable IDs, room-to-lane binding and
  deterministic application results from log order.
- Hosted owner runtime: serve from the reducer, publish state only after the
  archive gate, and recover state and replay position together. The existing
  `store.rs` remains the local-mode path, never automatic failure fallback.
- `server.rs`, `main.rs`, `handler.rs`: explicit hosted configuration and the
  real create/send/history caller path.
- Integration tests: real broker, actual Cowchat client calls, crash/rebuild,
  lost ACK, conflicting retries and refused mutations outside the first slice.

The current store also atomically creates actor deliveries, and other handlers
mutate grants, invitations, subscriptions, votes, elections and work claims.
Inventory these before enabling the new mode. A first slice must explicitly
refuse unsupported writes instead of silently keeping their authoritative
state only in SQLite. Each supported mutation must enter the replay contract
before the hosted replacement is complete. Hosted reads use the in-memory
projection described below; no room content goes through Labs Postgres.

## Keep the boundary small

Use the existing CBQS client and the PR55/PR59 fence stack. No new per-lane
fencing, generic adapter registry, automatic SQLite fallback, or cursor-generation
scheme without a concrete caller need. Service secrets/API keys must not be
serialized into log commands. Owner-stream configuration and room authorization
must be checked before appending, with replay preserving the relevant identity
and membership decisions.

Do not silently weaken takeover: the local experiment's OS lock and epoch
journal are not a distributed ownership service. Select and implement one
durable cross-host owner/epoch allocator, and archive/recovery backed by CBFS,
before claiming hosted worker failover. Those are required integration work;
broker replication is the explicitly deferred part.

## Verification and completion

Start with one owner stream, two rooms and two independently connected clients.
Verify that room lanes stay isolated, replay reconstructs ordering and retry
results, CBQS unavailability cannot cause a local-only success, and interrupted
archive capture withholds success. Exercise a takeover with a delayed stale
append and rebuild after deleting the projection. Run appropriate formatting,
clippy and caller-level tests for each slice.

Then extend replay to membership, actor work, votes/elections and other supported
durable mutations, test recovery beyond retention using CBFS, and measure the
actual finalized-authority path. The earlier JSON-snapshot latency figures
cannot be reused as measurements of this path.

This document records implementation scope, not completed behavior. Existing
PR59/60/61 evidence remains fixture-level until the caller paths above pass.

## Implemented foundation, September 21

`crates/cowchat-server/src/room_log/` now contains the deterministic create-room
and encrypted-message reducer, plus an optional `cbqs` transport using the actual
CBQS SDK pinned to PR59 commit `7f0fb216`. Recovery verifies the provider-signed
checkpoint chain and every payload digest before decoding commands. CBQS only
supports one lane per cursor, so recovery enumerates lanes, reads each with
bounded credit, and merges by global stream sequence. A missing prefix, expired
history, stale writer, verification failure or uncertain transport result retires
the session. It does not fall back to SQLite or blindly resend. A caller that
already holds a verified prefix can now request only the suffix after that
checkpoint. Record/byte bounds apply to the suffix; lane enumeration has a
separate 16,384-room-lane bound. A missing unarchived segment still refuses
recovery. Checkpoint handles cannot be deserialized from untrusted JSON. Archive
segments now preserve raw CBQS headers, ciphertext payloads and signed receipts.
`restore_archive` uses the same signature/linkage/digest verifier as broker
replay before recreating a checkpoint. The bounded format and optional CBFS
batch publisher are implemented; their owner-runtime/caller integration is not.

This is not connected to `CowchatServer` or its public handlers yet. The reducer
is an in-memory projection; it does not persist snapshots or provide archive
durability. No cross-host epoch allocator or live-tail subscription is
implemented. No live-chain claim is made:
the broker tests use actual WebSockets/RocksDB with a fixture node snapshot.

Verified commands:

```sh
cargo test --locked -p cowchat-server --features cbqs-test room_log --lib
cargo test --locked -p cowchat-server
cargo fmt --all -- --check
cargo clippy --locked -p cowchat-server --all-targets --features cbqs-test -- -D warnings
```

The focused suite has fifteen tests: five reducer cases and ten actual-broker
cases covering interleaved room lanes/retries/fencing, total retention expiry,
replay bounds/known-tip rollback, credit replenishment above 1 MiB, and ciphertext
tampering between the broker and client. The two suffix tests expire a verified
prefix and successfully resume with a one-record limit, then verify that a
checkpoint cannot bridge a missing unarchived segment. Two archive tests restore
a prefix from serialized wire records after total broker retention, join it to
the live suffix, and refuse tampering/truncation/reordering. This is verification
of archive bytes, not proof of CBFS persistence. A deterministic cancellation
test holds an actual committed Append ACK, drops the caller future, and requires
immediate local refusal of another append before any network I/O. The regression
fails with cancellation retirement disabled and passes with the drop guard.

Ordinary Rust client sends now assign a message ID before sending, and
`prepare_message` lets callers retain the ID and ciphertext for
`append_prepared_message` retries. A real TCP test serializes the prepared
payload, reconnects under the same authenticated identity, gets the original
receipt and rejects reencrypting under the same ID. The default server suite
has 195 tests; the default workspace has 276.
The optional SDK graph requires Tokio 1.50. The repository currently has only a
release workflow, so these are local checks, not CI results. Clean hosted builds
also need access to the pinned Cowboy Git dependencies.

## Caller paths that must be covered before enabling hosted mode

| Surface | Required treatment |
| --- | --- |
| `handler.rs` create/send/history/list/info/tip | Route through the active owner-stream and a recoverable projection. A retry must retain its first accepted timestamp and display name. |
| Room rename/destroy, invites/grants, subscriptions, actor work, votes/elections/tasks, thinking/presence | Log the durable mutation or explicitly reject it in the initial hosted slice. Current local handlers write SQLite directly. |
| `web.rs` HTTP invite redemption and blob upload | These bypass the frame dispatcher. They need the same hosted gate; blocking only WebSocket mutations is insufficient. |
| `server.rs` startup, authentication and agent identity claims | Restore before accepting traffic. Keep credentials out of log records; use a stable authenticated principal binding. Current room ownership stores an API bearer key locally. |
| Webhook/actor delivery workers and retention/blob sweepers | Start only supported workers. Local retention must not silently purge the hosted projection or archive state. |

The existing room mutation guard is synchronous. Do not hold it across CBQS or
archive network awaits; serialize commands at the owner-stream runtime instead.

Claude approved the foundation and the implementation simplification at room
sequence 137: serve hosted reads directly from `OwnerState`, rebuilding it from verified logs
and authenticated archive checkpoints. That avoids maintaining another SQLite
projection beside the reducer. It does not remove the durable append-intent,
archive or cross-host ownership requirements. It is not wired into handlers yet.

## Archive integration constraint under investigation

The success/visibility gate remains durable archive capture of a checkpoint
batch. Claude proposed moving it to CBQS commit at sequence 140, then withdrew
the safety equivalence at 142 after the counterexamples at 141: stopping new
appends does not stop time-based retention of already acknowledged records, and
a single broker disk is not a second durable copy. A weaker committed/retained
product contract would require Chad's explicit choice. First measure the actual
CBFS batch commit path; do not add app-level replication to avoid an unmeasured
latency concern. Pending commands must not enter served history or live events
before the selected durable gate.

The current CBFS SDK's `Volume::put` uploads object bytes and stages its manifest;
`Volume::commit` publishes through `CowboyHttpManifestRegistry` and the relayer's
`/cbfs/v1/manifests/commit-v2` path. Do not equate a staged object, local spool or
test registry acknowledgement with a durable, discoverable archive. Check the
actual authority/finality and garbage-collection paths before selecting the
archive ACK boundary. Calling a full volume commit for every message could
reintroduce the per-message chain dependency excluded from this design. The
runtime batching cadence and caller recovery integration still need implementation.
Archive verification currently uses the SDK session's chain-anchored provider
key history. Recovery of receipts older than that key history must fail closed
until a verified historical-key lookup is supplied.

`cbqs-archive-reassignment-spec.md` in the parent workspace is explicitly parked
and unreviewed, and describes older v1 standard/fast streams. It is background,
not an approved dependency or evidence that broker archiving exists.

## CBFS checkpoint publisher

The optional `cbfs-archive` feature pins the existing CBFS SDK to `d8ddaad0`.
`CbfsArchive` requires a clean private volume opened against authenticated root
authority. Missing authority is an error, including on an empty volume; initial
provisioning must explicitly establish its zero root. The opener reconciles any
pending SDK journal before it loads the archive head or stages another batch.

Each complete verified range becomes an immutable object named by its final
checkpoint ID. One manifest commit publishes that object together with a
`head.json` containing the archived sequence and checkpoint. Publication only
extends the current head without a gap. Exact republishing is a byte-identical
no-op; a different encoding under the same checkpoint ID is refused. Stale roots,
failed or cancelled publications retire the writer until authoritative reopen and
reconciliation. The publisher obtains range bounds from private signed proof,
not the caller-mutable decoded records. Reads still require CBQS signature and
digest verification; the discovery head does not replace it.

The opt-in subprocess tests use three actual CBFS nodes (private volume, 2+1
erasure coding), production QUIC/shard handlers and standalone Sled CAS metadata.
SDK test support is used only to isolate journal directories. Tests cover exact
republishing, SIGKILL/restart of every storage process followed by cold recovery
after broker retention expiry, a lost reply after the metadata commit, a new
batch after that reconciliation, stale writers and backward-head refusal.
All 18 log tests pass; the next-batch regression fails when opener journal
reconciliation is disabled. The 276 default workspace tests, formatting and
strict all-targets clippy with both test feature selections also pass locally.
They do not prove simultaneous cross-host writer races, chain finality, long-term
GC behavior or production archive latency. Loopback node auth is explicitly
test-only. The node binary must be built from the same pin; tests fail when its
path is not provided.

```sh
# In a CBFS checkout at d8ddaad0dee6a0b8b57bd7308376208446ced012:
cargo build --locked -p cbfs-node
# In this Cowchat checkout, use that binary's absolute path:
COWCHAT_TEST_CBFS_NODE=/absolute/cbfs/target/debug/cbfs-node \
  cargo test --locked -p cowchat-server --features cbfs-archive-test room_log --lib
cargo clippy --locked -p cowchat-server --features cbfs-archive-test --all-targets -- -D warnings
```

Hosted routes remain disabled. Bounded discovery/replay of a multi-segment archive,
durable append intents, cross-host ownership, actual handler wiring and measured
finalized-authority batch latency remain required work.
