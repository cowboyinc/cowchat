# Dashboard/Cowchat implementation cohort

Snapshot: 2026-09-13, after Claude's approvals through `dashboard-impl` message
1670. These are reviewed local components, not a deployed dashboard or a launch
verdict. No branch in this cohort has been pushed. UI remains with Patrick/Ryan.

## Code branches and proof

| Repository / worktree | Branch / reviewed code head | What this cohort proves |
|---|---|---|
| Cowchat, `.worktrees/cowchat-room-wake` | `jw/dashboard-cowchat-room-wake` / `09e74f4864f2c153212f390ec44b4ec4c907c14d` | Authenticated encrypted HTTP rooms, durable mention wakes, retry-safe reply sink, owner/actor/builder enrollment, local subscription lifecycle/revocation, scoped HPKE delivery, and door notification predicates. |
| Gateway, `.worktrees/gateway-room-wake-forwarding` | `jw/dashboard-room-wake-forwarding` / `6afd16626695464eb0b41e3a8c55d423c68def82` | Existing generic runner proxy carries the three Standard Webhooks verification headers; actual dispatch/proxy fixture, without provider-specific handling. |
| Harness, `.worktrees/harness-cowchat-wake` | `jw/harness-cowchat-wake` / `8a53482675209c58559a324bde0fd879ef75fd88` | Actor-bound wake handler verifies before key access, accepts an independently authorized transient runtime lease, verifies/decrypts its trigger, and emits/reconciles one authenticated encrypted reply. Runtime key provider remains an interface. |

Paths are relative to `/Users/chadd/dev/cowboy`. The three worktrees were clean
at the code heads above. Subsequent documentation commits can advance a branch
without changing these code/proof snapshots. Original checkouts and the frozen
route worktrees remain separate and untouched.

Cowchat's ordered local commits after baseline `3fc2e33`:

| Commits | Delivered boundary |
|---|---|
| `17dac99`, `d942348` | Durable mentions and atomic subscription recovery. |
| `2b230e1`, `65752c2`, `aca8682` | Wallet owner bootstrap, signed encrypted append/history and seated mention subscriptions. |
| `8b12917`, `9e0275e`, `29d170d` | Actor-controller certificate matching, release-pinned finalized-proof verification and enrollment with durable rollback floors. |
| `56c8a0c`, `c4a487b` | Signed client requests, exact trigger read, fresh encrypted candidates, exact retries and authenticated conflict winner. |
| `b3073d2` | Separate-process local wake/reply recovery proof. |
| `de025e4` | Signed subscription update/delete/repair, durable receipts and revision-fenced webhook outcomes. |
| `99301cb`, `4694d55` | Background verified actor-authority refresh and authenticated absence/revocation; no per-message chain calls. |
| `b73173d`, `a5f98ab` | Scoped recipient HPKE envelopes and real wallet-issued builder enrollment/delivery/posting. |
| `09e74f4` | One role-derived door notification predicate across live enqueue, backlog, repair and dispatch. |

Gateway's additions are `75986c8` and `6afd166`, atop existing prerequisite
`c036519` (runner header/query/stream forwarding). Harness's additions are
`4856e05` and `8a53482`, atop `8241047`. Its two Cowchat dependencies pin real git
commit `c4a487b5592f42ec93d9041e44194c67c2ebe9ad`; no local Cargo patch remains.
That commit is an ancestor of the Cowchat branch head. Eventual publication must
make it fetchable before publishing the harness dependency. Actual upstream PR
bases, current CI and cross-repo publication order must be rechecked when shipping
is authorized; a local build is not evidence of published availability.

## Evidence and limits

The latest Cowchat battery passes 150 server library tests, 71 integration tests,
the binary tests, strict all-target Clippy and formatting on Rust 1.93, offline and
locked. The preceding builder/crypto battery also passes all 19 crypto integration
tests. Claude independently reviewed and ran the corresponding batteries.
Gateway's 13 proxy tests and harness's 163 library / 101 route-parity tests passed
on their code heads; those repositories did not change in the later Cowchat cuts.

The actual three-process regression on the door-filter code passes: room service
and durable worker -> generic gateway dispatch/proxy -> actual harness handler ->
authenticated encrypted room reply. Service restart with a queued wake and harness
crash after append before acknowledgment produce two handler executions and one
visible reply, retaining the same delivery ID and wake bytes. Forged wake returns
401 before key access; no consensus calls or provider API calls occur. The runtime
and room credentials are explicitly public fixtures. This is not real actor key
release, billing, external-provider delivery or production logging proof.

Local report:
`/var/folders/1x/y2_zywnj4db6jsvnbkm9gbp00000gn/T/cowchat-m-echo-sn81yasq/report.json`.
Dispatch `b1eaf9da-a9c2-4afa-9c5c-9bcc11058901`; trigger
`20000000-0000-4000-8000-000000000001`; reply
`7cc9ffb4-1908-8f50-8f37-c3284b540f99`. The driver cleaned up its processes.

Separately, the builder test uses real HTTP certificate factories, not SQL-seeded
credentials: owner enrollment -> wallet-issued builder enrollment -> owner-signed
HPKE envelope -> builder retrieval and local verification/decryption -> encrypted
builder-attributed append. Public test wallets exercise the production service
path. Automatic identity renewal and recipient-private-key recovery remain open.

Door tests deliberately seed authenticated door context; there is no production
door enrollment factory yet. They prove notification policy, not an external
bridge. A destination worker must filter every record it reads in a backfill
range, maintain its own outbox/origin state and handle uncertain provider results.
The current actor mention handler is not a door executor. No ordinary member
can append system records; the system writer in that routing test is a fixture.

## Current boundaries and remaining plan reconciliation

Chad's latest instruction keeps these flows off consensus. All Track R and Unit B
implementation **and activation** remain frozen, including old RouteInvocation
admission and the ledger worktrees. Preserve their dirty files. The older plan's
proposed thin Track R continuation is superseded where it conflicts with that
instruction. Do not infer a provisioning exception from the existing code.

The next useful ungated check is a bounded logging/persistence review of these
actual room boundaries. Production checkpoint provisioning needs an authenticated
release anchor and deployment process; fixture checkpoints do not establish one.

The separate CBSS room-session release design is directionally approved only.
Concrete specification/golden-vector review and Chad's ruling on the new named
access-authority trust boundary and initial provisioning remain outstanding.
The existing CBSS job release still requires a real actor-owned assigned job; it
must not be weakened or given invented job IDs. A production actor/door key
provider and real enrolled-key wake proof follow that decision.

The cohort does **not** establish completion of the broader non-UI launch plan:

- CBQS transport adapter and independent CBFS archive, including hosted rooms;
- Python room SDK, off-consensus runtime backend, packaging/template/doctor;
- Dashboard room provisioning/lifecycle and generic catalog-driven gateway setup;
- Private harness room tools and room-mode session/run-event/memory persistence
  controls, including the Cattle Guard integration;
- Until-revoked access with automatic credential rotation and private-key recovery;
- Door enrollment, destination worker/outbox and authenticated system records;
- Production checkpoint, deployment, restore/rollback, pin publication and live
  environment acceptance.

Some earlier work on other branches may contribute to these items. It requires
its own current inventory and integration proof; it is not covered by the three
heads above. The reset section of the root implementation plan still explicitly
includes a minimal WP3 room mixin and preserves the broader launch requirements.
“Door filter unit complete” or “stage 1 complete” must not become “everything but
the ship checklist is done.” Claude coordinates the root plan update and next
bounded priority from this snapshot.

Contracts and recipes: [seated owner](seated-owner.md),
[actor enrollment](actor-enrollment.md), [builder enrollment](builder-enrollment.md),
[key envelopes](room-key-envelopes.md), [door filters](door-notification-filters.md),
[local three-process proof](local-m-echo.md), and
[key-delivery design](room-key-delivery-design.md).
