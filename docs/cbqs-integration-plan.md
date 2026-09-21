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
batch publisher are implemented. The owner runtime now composes these pieces;
authenticated caller integration is described below. Hosted CLI bootstrap is
still pending.

`CowchatServer::new_hosted` now explicitly connects a recovered runtime to the
restricted authenticated handler surface described below. The reducer
is an in-memory projection; it does not persist snapshots or provide archive
durability. The owner runtime supplies the archive and allocation gates, but
no live-tail subscription is implemented. No live-chain claim is made:
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
archive or cross-host ownership requirements. The initial authenticated handlers
now use one shared committed view of that projection.

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
authority and a durable discovery head. Explicit new-volume provisioning writes
a genesis head once. Normal open always requires that head, even for empty
history: CBFS returns to zero root after deleting its last file, so zero root
alone does not prove freshness. The opener reconciles any pending SDK journal
before it loads the archive head or stages another batch.

Each complete verified range becomes an immutable object named by its final
checkpoint ID. One manifest commit publishes that object together with a
`head.json` containing the archived sequence and checkpoint. Publication only
extends the current head without a gap. Exact republishing is a byte-identical
no-op; a different encoding under the same checkpoint ID is refused. Stale roots,
failed or cancelled publications retire the writer until authoritative reopen and
reconciliation. The publisher obtains range bounds from private signed proof,
not the caller-mutable decoded records. Reads still require CBQS signature and
digest verification; the discovery head does not replace it.

`recover` walks backward from the authenticated head through receipt links,
then verifies every segment forward from genesis using the existing CBQS
signature/linkage/payload verifier. Discovery fields are explicitly unverified
until that second pass. It checks the final verified checkpoint ID and sequence
against the head, refuses a changed authoritative root, and returns no partial
records on failure. One deadline and total segment/record/encoded-byte budgets
bound the complete recovery. This first implementation fails closed when complete
history exceeds those budgets. Runtime writes also enforce those total budgets;
snapshot compaction and serving a bounded tail of a larger history remain future work.

The opt-in subprocess tests use three actual CBFS nodes (private volume, 2+1
erasure coding), production QUIC/shard handlers and standalone Sled CAS metadata.
SDK test support is used only to isolate journal directories. Tests cover exact
republishing, SIGKILL/restart of every storage process followed by three-batch
cold recovery after broker retention expiry and live-suffix replay, a lost reply
after the metadata commit, a new batch after that reconciliation, stale writers
and backward-head refusal. Additional cases refuse a tampered/missing middle
segment and enforce total byte/record/segment limits, including exact-limit success.
The next-batch regression fails when opener journal
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

The default CLI still starts local mode. Explicit `new_hosted` startup now exposes
the restricted frame surface below; production provisioning, deployed cross-host
ownership proof and measured finalized-authority batch latency remain required work.

### Ownership allocation boundary

PR59's fence persists only a monotonic epoch floor; it does not bind that epoch
to one holder. A real-broker test gives two different authorized holder keys the
same epoch, fences both successfully, and observes both appends succeed. Unique
epoch allocation is therefore an independent prerequisite, not supplied by the
fence. Expanding fence wire/session semantics is outside this Cowchat slice.

The proposed promotion operation compares an expected epoch, records a stable
writer ID and retry claim ID, then advances the epoch once. A lost response must
reconcile that same claim; a CAS loser must not automatically compete for the next
epoch. Allocation alone permits no appends: a winner must receive the matching
fence ACK before recovery and serving. Test a winner dying before that ACK, followed
by higher-epoch takeover and rejection of the abandoned epoch.

The selected implementation uses a separate private CBFS control volume for rare
promotion CAS operations. A different object in the archive volume would still
share its manifest-root CAS and would not isolate promotion from archive writes.
Production CBFS publication uses chain-backed manifest stage/finalize, not a fast
off-chain per-object CAS. Promotion and archive latency must be measured separately
on actual authority before claiming acceptable failover or chat latency.

Pre-append intents remain local, durable and batched. This replaces the earlier
cross-host-intent proposal after the case analysis at room sequences 161/164:
committed commands reconcile through verified broker/archive replay, while an
uncommitted operation is retried by its caller with the same ID AND ciphertext.
An unarchived operation was never acknowledged or visible to other participants.
Cross-host intent storage adds a manifest commit without strengthening the agreed
archive-before-success contract. This does not permit skipping an unarchived
history gap. Actual frontend retry callers must retain their prepared payloads;
assigning IDs inside the SDK alone is insufficient.

`ownership.rs` now implements that promotion CAS using the existing CBFS SDK.
`WriterRegistry` loads a versioned, bounded control record and compares the
expected epoch before advancing once. Same-claim/same-writer retries return the
same epoch/root; identity changes and stale expectations fail closed. Uncertain
or cancelled claims retire the registry until reopen. The successful receipt is
not deserializable and grants no CBQS session. A worker still needs a matching
signed fence ACK before it can recover, append or serve.

Control provisioning writes an explicit epoch-zero record once for a genuinely
new registered volume ID. Normal startup requires that record; it never infers
epoch zero from an empty root. The application provisioning workflow must keep
`initialize_new_volume` separate from recovery. The runtime now
enforces different control/archive volume IDs and binds writer identity to a unique
worker incarnation/grant holder key, rather than a reusable hostname. Replacement
workers promote with new identities; only retries of the same claim reuse its ID.

Real-node tests cover claim retry, a lost response after metadata CAS, a claimant
disappearing before fencing followed by higher-epoch takeover, stale/wrong-identity
claims, and refusal after control-record deletion. A separate test removes every
archive object and confirms reopening fails even though the authority root is now
zero; this regression failed before explicit genesis-head provisioning.

The promotion race launches two actual worker subprocesses with isolated SDK
journals against production CBFS node processes. A loopback metadata fixture
holds both prepared commits at a barrier with the same predecessor, then runs
the actual standalone Sled CAS. Exactly one succeeds; cold readback checks its
epoch, identity and root. The ignored `claim_process` test is the subprocess
entry point and is executed twice by this parent race test. This proves local
cross-process competition; deployed host failure, chain-finalized authority,
provisioning/caller wiring and measured latency remain outstanding.

The complete focused suite now passes 26 checks with one ignored subprocess
entry point exercised by the race. Formatting and strict all-targets clippy with
`cbfs-archive-test` pass. The prior 276 default workspace tests also passed; these
new allocation/archive changes are behind the optional feature.


### Owner runtime and local pending batches

`runtime.rs` now composes promotion, the fenced CBQS session, CBFS archive and
an in-memory `OwnerState`. `WorkerIncarnation::fresh` generates a new holder key,
writer identity and stable claim ID. Its consuming `bind` checks the allocated
writer/claim/epoch/stream against the actual fenced session and its holder key.
Runtime startup also requires distinct control/archive volume IDs and a journal
bound to the same owner/stream. It verifies archived history, captures any
surviving broker suffix durably, reconciles pending command receipts, and only
then makes the projection available. A missing log segment still stops recovery.

`intent.rs` reuses SQLite transactions for one worker-local pending batch. This
is not a hosted room projection. The private journal stores exact prepared
command IDs, ciphertext and lanes before appending, with finite record/byte
limits, a stream identity binding and an exclusive process lock. Blocking disk
operations run outside Tokio's executor threads. DELETE journal mode with
synchronous EXTRA/fullfsync provides atomic durable transactions. The batch is
cleared only after verified replay, archive publication and reducer application.
A replacement on another host needs no copy of this journal; uncommitted sends
remain the caller's responsibility to retry with their prepared ciphertext.

The runtime serializes batches through its mutable API (the handler must use
an async queue/mutex). Room lanes are allocated by stable owner/room key. Exact
retries return the original receipt without another append or fanout event;
changed ciphertext under an existing ID conflicts. Successful `CommittedBatch`
values include only newly applied records for post-archive fanout. Failed or
cancelled work retires the whole runtime, including reads, until fenced recovery.
No API key or plaintext message body belongs in an intent or log command.

The runtime slice initially passed 33 focused checks plus the separately invoked process-test
entry point. New real-node runtime cases cover two-room batches and cold rebuild,
held/cancelled archive publication, lost publication replies, partially appended
batches, mismatched incarnations/holder keys, and journal locking/identity/bounds.
A duplicated file-description test covers fork-to-exec lock inheritance: explicit
unlock is required when dropping a journal. Removing that unlock or the runtime's
cancellation retirement makes the corresponding regression test fail. The parallel node fixtures assign
unique test addresses outside the usual client ephemeral range to avoid both
QUIC client collisions and a sibling's not-yet-bound/restarting port.
These are local runtime checks, not authenticated client or deployed-host proof.

The initial authenticated handler and reconnect wiring is implemented below.
Actual frontend retries must retain their prepared IDs/ciphertext. Finalized
promotion and archive ACK latency, provisioning, compaction, and actor/gateway
integration remain unfinished. Nothing in this slice authorizes a merge or deployment.


### Authenticated hosted caller slice

`CowchatServer::new_hosted(config, recovered_runtime)` selects the hosted backend
before constructing any room workers. This is an explicit embedding API; the
ordinary CLI continues to start local mode and does not provision/promote a
hosted runtime. The initial slice binds the server's primary credential to one
stable runtime owner ID. Other valid server credentials do not acquire access to
that owner's rooms. Keyless access, no-auth and public signup are refused. Raw
credentials stay in local authentication configuration and never enter commands.
Durable multi-credential membership, invitation and actor grants are later work.

`hosted.rs` handles private encrypted creation, transient connection join/leave,
encrypted send, history, room info/tip/list and owner-scoped agent listing. Every
other frame operation returns an explicit unsupported error. The hosted HTTP
router refuses all `/api/` storage/administration routes, including invitation
redemption and blobs, while `/ws` uses the authenticated frame handler. The local
HTTP administration/board API is not a hosted frontend yet. Startup skips local
webhook/retention/blob workers, and VoteManager does not restore SQLite timers.
Hosted room/message rows are never written to the local Store.

One `OwnerView` shares the actual in-memory projection under a short synchronous
read/write lock. Existing reconnect and join authorization reads it without
awaiting storage or the async writer queue. Readers can see the previous committed
state during an in-flight batch. The runtime takes the projection write lock only
after archive publication; it retires the view before releasing that lock if
reduction fails. Cancellation/error retires both writer and shared readers.
No synchronous room/lifecycle/projection lock is held across a network await.
The async owner mutex remains held through post-commit fanout to preserve order.

The Rust client can retain `prepare_hosted_room`/`create_prepared_room` payloads
with a UUID room ID, in addition to the existing prepared message ID/ciphertext.
Hosted creation retries bind that ID to the original authenticated creator and
creation body. Local servers retain their existing server-assigned room IDs;
prepared creation idempotency is a hosted capability. The first hosted slice
explicitly rejects public/unencrypted rooms, descriptions and parent relationships.

The focused suite now passes 37 checks, plus the promotion subprocess entry
point. Three new tests exercise the actual authenticated TCP connection loop and
HTTP/WebSocket router with production CBQS/CBFS storage fixtures. They hold
archive publication and assert no send ACK, message event or history visibility;
a second client reads prior history and reconnects its live stable identity
without losing membership while the writer is busy. Release makes one message
visible. Exact retries emit no duplicate event, changed ciphertext conflicts,
and a fresh runtime/local-auth database recovers the original room/message receipt.
A lost archive reply retires every authenticated read until recovery. Tests also
cover a valid but unbound credential, unsupported mutation, HTTP bypass refusal,
WebSocket append read through TCP, and absence of hosted room/message rows in SQLite.
The listener fixture owns its connection tasks; it exercises production startup
construction/handlers/router, not the long-running daemon shutdown lifecycle.

The public history test fails when projection application is deliberately moved
before archive publication. The cancellation test fails if shared-reader
retirement is removed. Both pass with their guards restored. All 276 default
workspace tests and strict feature-enabled all-targets server clippy pass locally.
This remains fixture-backed authority, not a live finalized-chain or deployed
multi-host result. CLI provisioning, the dashboard's prepared-request retry path,
complete membership/actor/gateway behavior and measured durable ACK latency are
still required before hosted launch.
