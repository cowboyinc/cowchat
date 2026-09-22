# CBQS as the Cowchat room log

Source assessment and local experiments, September 21, 2026. This is a candidate for the
multi-server launch investigation authorized by Chad in `dashboard-cowchat-v2`, not a claim
that the current Cowchat implementation already supports failover.

**Recommendation:** CBQS is a plausible ordered transport for several Cowchat
servers. Interactive append latency is an engineering/measurement question, not
an inherent chain-finality barrier. However, moving messages into CBQS alone
would not make Cowchat stateless or make the underlying broker highly available.
Keep the existing client and actor interfaces while proving the storage change
in a small experiment before replacing the current SQLite authority.

## Sources

Fetched source pins, not deployed-network claims:

- [CBQS devnet `2eb933c`](https://github.com/cowboyinc/cbqs/tree/2eb933cab43a9aa9f6cc2c393666cf18353b42c3):
  `crates/cbqsd/src/transport/connection.rs`, `transport/mod.rs`,
  `transport/chain.rs`, `store_v2.rs`.
- [Node devnet `dd3b3c9`](https://github.com/cowboyinc/node/tree/dd3b3c9d475635df8fe8d0620acf9dd230c34588):
  `execution/src/cbqs_v2/{params,account,stream}.rs`,
  `types/src/constants.rs`.

CBQS's README contains older standard/fast and long-poll descriptions. The
current V2 dispatch waits for durable batches and the current transport is
WebSocket; use the pinned implementation for these conclusions.

## Four readiness questions

| Question | Current evidence | Implication |
|---|---|---|
| Append latency | `handle_request` parks an append until `commit_open_batch` completes a synchronous RocksDB write. The chain parameter defaults to a 20 ms batch window and 1,024 records. The periodic ticker checks batch age; at low traffic its timer component can be roughly 20–40 ms before disk, scheduling and network time. | Sub-100 ms warm appends are plausible, now measured locally at roughly 40 ms ACK p99 under the default setting. The batch timer is tunable (minimum 1 ms), with a throughput/fsync tradeoff. Do not weaken durability to claim speed. |
| Throughput and cost | Default admission ceilings are 1 MiB/s appended and 4 MiB/s delivered per stream, subject to stream/grant limits. Rent is elapsed blocks × locked account rate × active streams; default rate is 80,000 atomic units per stream/block. No append transaction or per-append escrow debit is in this path. | Limits are configurable, not measured capacity. Default rent is 0.00008 CBY/stream/block; stream creation separately burns 5 CBY, and a new account requires at least 1 CBY top-up. A literal stream per room adds chain state and cost even for idle rooms. Room lanes within an owner stream deserve comparison before fixing the mapping. No live tariff or USD price is asserted. |
| Fanout latency | Commit broadcasts a stream wake to watching connections. The WebSocket loop immediately pumps on that wake and continues draining while there is work. The 500 ms tick is housekeeping/backstop. | There is no mandatory 500 ms polling floor. Give each Cowchat server an independent replay cursor; a shared consumer group would distribute messages instead of giving every server the full log. N tailers still multiply delivered bytes and must replenish credit. |
| Concurrent ordering | Connections in one broker share one mutex-protected stream accumulator and sequence space. Durable commit assigns the sequence range before resolving append waiters. | Concurrent producers through that authority get an order. This is not consensus between independent brokers and is not compare-and-set for Cowchat turn tokens. Cowchat must validate competing commands in log order and return the accepted/rejected result after application. |

One additional latency concern is concrete: the request path calls
`fetch_snapshot`; `ChainClient::snapshot` refreshes a held snapshot after a
250 ms successful-cache window and waits for that RPC. Its HTTP timeout is five
seconds. Warm steady-state percentiles must include refresh misses and degraded
node behavior. Serving an allowed held snapshot while refreshing in the
background is a possible targeted optimization, subject to preserving the
existing authorization/freshness rules.

## Work that a faster append cannot replace

1. **Cowchat state must replay.** Room creation and membership, actor enrollment,
   stable message IDs, turn-token checks, votes/elections, work claims and
   completion must derive from ordered validated commands. Local validation
   before append alone races another server. Retry may append the same command
   again after a lost acknowledgment; its stable command ID must produce one
   accepted effect. CBQS append itself is not a replacement for Cowchat's
   existing idempotency contract.
2. **History must outlive retention.** The default CBQS retention ceiling is
   seven days and 1 GiB per stream. A new server cannot rebuild arbitrary room
   history from an expired log. CBFS for attachments alone does not solve this:
   the design also needs retained event archives and/or recoverable snapshots
   with an explicit history/replay boundary before deleting any log prefix.
3. **Broker failure is distinct from Cowchat-server failure.** The reviewed
   commit persists to one RocksDB store and shares ordering locks inside one
   process. It does not implement replicated-log consensus between brokers.
   Multiple Cowchat servers can survive a Cowchat process failure while sharing
   that broker, but losing the broker host/storage still needs a separate
   recovery mechanism. Adding a second broker endpoint alone is insufficient.
4. **External effects remain at least once.** Two servers replaying the same
   event must not independently wake/execute an actor or forward it externally
   without a durable claim/fence. A room log orders decisions; it does not make
   arbitrary outside APIs transactional.

These are implementation requirements, not arguments against CBQS. They should
be explicit in the launch definition and tested individually. The experiment
changed no Node code. A separate CBQS policy-fence fix is described below.

## Earlier availability plan

The workspace's `cbqs-implementation-plan.md` (August 19, baseline `e23f9cd`,
reviewed in `cbqs-update` at sequence 33) already identified this boundary:
COW-3299a selects an availability contract; COW-3299b proves stale-writer
rejection and monotonic signed history under takeover. Its simpler HA candidate
is **fenced active/passive over supported replicated storage**. A new
replicated-log consensus implementation is not the only possible solution.
COW-3300 separately requires a consistent checkpoint and an isolated restore
exercise. Those are planned acceptance requirements, not current HA evidence.
The plan predates today's V2 transport, so its Standard/Fast and long-poll
implementation details must not be copied into new work.

For Cowchat, evaluate recovery at two distinct boundaries: (1) a Cowchat worker
fails while the broker remains healthy; (2) the broker host or storage fails.
The first experiment exercises a fixture of the first boundary and single-store
broker restart.
The second needs an explicit storage/fencing contract and a separate failure
drill. Neither a second process on the same disk nor restored old backups prove
that every acknowledged append survives broker-host failure.

The more specific `dashboard-cowchat-spec.md` room-service section already
proposed **one fenced writer per room plus standby**, a durable append intent
before CBQS append, lost-ACK reconciliation by stable message ID, and success
only after log commitment plus encrypted archive capture. That is the useful
availability contract to retain. Its older chain-heavy membership/seat design
is not being reintroduced by this experiment.

## Local results and the fence fix

[CBQS PR60](https://github.com/cowboyinc/cbqs/pull/60) preserves the devnet
experiment and exact JSON readout. Two independent cursor tailers each received
1,130 records across six 4 KiB profiles. ACK p99 was approximately 40 ms at the
20 ms default batch setting and 6 ms with the setting changed to 1 ms. A
synthetic 50 ms snapshot RPC delay yielded 74 ms ACK p99 and 95 ms tailer p99.
These are local debug-build observations on Apple M5 Max, **not bounds** or
production latency promises. The five-second RPC timeout remains relevant.
Those measurements use devnet `2eb933c` and JSON snapshot fixtures; they do not
measure PR55's finalized-authority production path.

The test worker fixture recovered a committed message after losing its ACK,
kept exact retries to one physical append, serialized competing expected-turn
commands, blocked success while archive capture was unavailable, and rebuilt
three committed messages after actual worker and broker SIGKILL. Broker restart
used the same RocksDB disk. Allocation used a same-host OS file lock and durable
epoch journal; the archive was local test storage. This is not production
Cowchat, distributed allocation, CBFS archival, or broker-host/storage failover.

A negative control held an old writer's Append frame in transit, promoted a
new writer at the application level, then released the old frame. Both appends
committed: an opaque application epoch alone does not fence the broker.

Existing [CBQS PR55](https://github.com/cowboyinc/cbqs/pull/55), pinned at
`fdd0a94ad7f570e6a2f28aafa8668e5383cd03d1`, provides finite policy-epoch grants
and a signed broker fence with an under-lock admission recheck. We found one
remaining ordering hole: its fence ACK could precede commitment of an old
writer's already-admitted batch. [PR59](https://github.com/cowboyinc/cbqs/pull/59)
at `7f0fb21` drains that batch under the existing barrier before signing the ACK.
The regression failed on the base and passed with this fix; a real socket test
also holds an old Append until after the new epoch ACK and verifies typed
rejection with no extra record. No new wire field or consensus protocol.

A subsequent process-fixture composition uses real finite-epoch grants and
waits for the signed fence before replay. It passes both fault schedules:
old worker killed after commit/before response, and old worker killed with
Append bytes still in transit. In the latter, the standby retries the pending
intent once and the delayed old frame is explicitly rejected. This strengthens
local integration evidence; it does not change the allocation/archive/host-HA
limits above. [PR61](https://github.com/cowboyinc/cbqs/pull/61), at `491699e`,
preserves the composition tests and their separate report/JSON.

The two necessary takeover guarantees remain distinct: the ownership service
allocates one winner per epoch; CBQS rejects earlier grants and resolves their
admitted appends before acknowledging the fence. Proof-derived native room
sessions bypass this off-chain policy fence, so a room-service writer using
this mechanism must use finite-epoch admin grants. Production ownership and
archive recovery still need implementation and their own failure drills.

**Align ownership with the fence's scope.** Package R epochs are per stream,
not per lane or room. Sharing one owner stream across many room lanes reduces
chain state and per-stream cost, but independent room owners cannot advance
their epochs independently: a room takeover would revoke grants for the other
rooms on that stream. The simple choices are one active writer per stream
covering all its room lanes, or one stream per independently failed-over room.
Prefer the former for the next prototype given the minimal-chain-state goal;
validate that shared failure/ownership boundary before making it a production
decision. A new per-lane fencing protocol is not needed for this prototype.

## Experiment contract

Use current V2 code, one broker and two Cowchat room workers with disposable
local caches. Drive two concurrent producers and independent tailers. Measure
append-to-durable-ACK and append-to-each-tailer p50/p95/p99 at interactive cadence
and sustained load, including snapshot refresh. Repeat with the batch setting
changed only if measurements identify it as the constraint.

Then kill a Cowchat worker after append but before reply; replay into an empty
cache; race the same expected turn token through both workers; retry the same
command ID. Both workers must converge on one accepted result and retain every
acknowledged message. Separately kill the broker to establish exactly what its
existing single-store restart recovers. Do not label that test host/storage
failover. Before calling the cache disposable for production, test recovery past
the retention boundary using the chosen archive/snapshot mechanism.

Local latency and fixture integration tests are reported above. Live funding,
production Cowchat failover, recovery past retention via CBFS, and broker-host
failover remain untested. The generic connector/gateway scope is unchanged:
Telegram is the first adapter, not the boundary of the product.
