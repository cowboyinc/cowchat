# Cowchat owner-stream integration

Chad's September 21 decisions, relayed by Claude in `dashboard-cowchat-v2`
at sequence 121 and confirmed by Chad in the implementation task:

- CBQS is the launch room-log authority.
- One stream per owner/tenant, rooms as lanes, one active writer per stream.
- Broker-host HA follows launch. Single-broker restart recovery is required.
- Connectors remain generic. Telegram is the first adapter; no second-provider
  priority has yet been relayed.

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
- `store.rs`: projection application and replay position committed together;
  retain existing local mode without treating it as automatic failure fallback.
- `server.rs`, `main.rs`, `handler.rs`: explicit hosted configuration and the
  real create/send/history caller path.
- Integration tests: real broker, actual Cowchat client calls, crash/rebuild,
  lost ACK, conflicting retries and refused mutations outside the first slice.

The current store also atomically creates actor deliveries, and other handlers
mutate grants, invitations, subscriptions, votes, elections and work claims.
Inventory these before enabling the new mode. A first slice must explicitly
refuse unsupported writes instead of silently keeping their authoritative
state only in SQLite. Each supported mutation must enter the replay contract
before the hosted replacement is complete. SQLite can remain a local projection
inside the Cowchat service; no room content goes through Labs Postgres.

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
the session. It does not fall back to SQLite or blindly resend.

This is not connected to `CowchatServer` or its public handlers yet. The reducer
is an in-memory projection; it does not persist snapshots or provide archive
durability. No cross-host epoch allocator, CBFS archive, live-tail subscription,
or recovery beyond CBQS retention is implemented. No live-chain claim is made:
the broker tests use actual WebSockets/RocksDB with a fixture node snapshot.

Verified commands:

```sh
cargo test --locked -p cowchat-server --features cbqs-test room_log --lib
cargo test --locked -p cowchat-server
cargo fmt --all -- --check
cargo clippy --locked -p cowchat-server --all-targets --features cbqs-test -- -D warnings
```

The focused suite has ten tests: five reducer cases and five actual-broker
cases covering interleaved room lanes/retries/fencing, total retention expiry,
replay bounds/known-tip rollback, credit replenishment above 1 MiB, and ciphertext
tampering between the broker and client. The default server suite has 194 tests.
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
