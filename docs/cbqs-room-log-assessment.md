# CBQS as the Cowchat room log

Read-only source assessment, September 21, 2026. This is a candidate for the
multi-server launch requirement relayed in `dashboard-cowchat-v2`, not a claim
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
| Append latency | `handle_request` parks an append until `commit_open_batch` completes a synchronous RocksDB write. The chain parameter defaults to a 20 ms batch window and 1,024 records. The periodic ticker checks batch age; at low traffic its timer component can be roughly 20–40 ms before disk, scheduling and network time. | Sub-100 ms warm appends are plausible, **unmeasured here**. The batch timer is tunable (minimum 1 ms), with a throughput/fsync tradeoff. Do not weaken durability to claim speed. |
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
be explicit in the launch definition and tested individually. No CBQS or Node
code was changed by this assessment.

## Smallest useful experiment

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

No latency benchmark, live funding test, multi-server integration or broker-host
failover test was run for this source assessment.
