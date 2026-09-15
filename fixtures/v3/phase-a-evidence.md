# M0 phase A: route execution feasibility

2026-09-10. **Phase A closed as designed-not-found; live probe not run.** A generic
private route-to-job admission bridge has not been found in the inspected
integration heads. That bridge is a design/implementation prerequisite; an
HTTP 200 from harness-serve is not a substitute for the required proof.
Chad chose Option A on September 10 (room message 36): a protocol
`RouteInvocation` admitted by node as an actor-owned assigned job, with gateway
as relay. M1B is active. The missing bridge is now designed work in WP2, whose
acceptance gate carries the live execution, release, debit, and settlement proof.
Actors and `offchain/` Runner bundles are authored separately; the built,
content-addressed bundle hash binds code/runtime at admission. This supersedes
the earlier one-source/two-sites packaging requirement.

## Exact evidence heads

These remote branch tips were read with `git ls-remote --heads origin devnet
main`. Node and gateway objects were fetched by exact SHA without switching
their worktrees. They supersede the earlier local-head snapshot only for this
report; no product pin or implementation-plan evidence pin was edited.

| Repo | Inspected integration head | Relevant result |
| --- | --- | --- |
| runner | `6f7ad42e1af242c90a7a89f5a3deb9f988905661` | No Rust `/_cowboy/route` or `RouteContext` implementation; ordinary job executors exist. |
| node | `bfde80767f302f8c24fb0f28aa911ee6cb8a5bbe` | Release purposes remain Read, Verify, ReadOrVerify. |
| cbss | `fd5bb4691989df5fa4bd50d1c4a12756b40c1759` | Release authorizer checks a real job and assigned runners. |
| gateway | `5db2d1e7433e97c15c071358bbddbe2644f9e732` | `proxy_to_runner` forwards body and route context; it does not itself create the required job assignment. |
| cowboy-protocol | main `4a04245d1ed66bec34c93b76f1062fec153a6b97`; devnet `d507db363dfec5d5f841bfdf22a6aa6aaa7ec14a` | Branch tips differ. Pin selection must be explicit, not inferred from branch names. |

Additional inspected source: local cowboy-harness `8241047` and node SDK
`38445e45` (its relevant SDK file unchanged by the later two node commits).
The live node delta adds nonce-floor handling and a test fix; the latest gateway
delta adds Transaction v2/pin convergence and actor-submit handling, while its
Runner forwarding function still lacks job admission.

## Producer and consumer boundaries

1. `gateway/crates/gateway-server/src/lib.rs::proxy_to_runner` resolves the
   explicitly pinned runner endpoint, serializes `RouteContext` to a base64url
   CBOR header, and posts the raw request to `/_cowboy/route`. A gateway routing
   test can establish this forwarding behavior. It cannot establish a generic
   executor, finalized assignment, CBSS release, or actor-job settlement.
2. `cowboy-harness/crates/harness-serve/src/server.rs` owns the known
   `/_cowboy/route` endpoint and parses `harness-api::AgentRouteBody`. This is
   the harness agent server. Pointing the generic actor probe there would test
   a different execution contract.
3. `runner/crates/runner-node/src/entry.rs` registers HTTP, LLM, MCP, Container,
   and optional CLOB Custom executors. Container can run arbitrary packaged
   code, but an executor registry entry is not a route admission path. A search
   of all Rust files at the runner integration head finds neither the route
   endpoint literal nor `RouteContext`.
4. `runner-node/src/node.rs::execute_job` receives an existing `JobSpec` and
   checks `verify=` before read-secret release or execution. It then releases
   referenced secrets and dispatches an executor. This is useful existing
   machinery once a real route-associated job is admitted.
5. The same file's `sign_and_strip_http_job` is existing **secp256k1 prehash**
   HTTP signature substitution for trading APIs. It is not the proposed
   Ed25519 `sign=` operation or the strict CBSS Sign release purpose.
6. `cbss/crates/cbssd/src/chain_authorizer.rs::HttpChainReleaseAuthorizer`
   verifies the runner's signed release request, reads `/job/0x{id}` and
   `/job/0x{id}/runners`, and checks chain authorization. A raw forwarded HTTP
   request does not supply that assignment.
7. `node/pvm/Lib/cowboy_sdk/runner.py::_RunnerAwaitable.send` emits an on-chain
   runner job; direct await raises `ContinuationLimitError`. Reusing this path
   with plaintext room input does not demonstrate off-consensus handling.

## Dependency prerequisite

Runner `6f7ad42` pins protocol `9bc2de3` and cbss `81b3a12`. CBSS `fd5bb46`
pins the protocol family to `4a04245d`. The newer protocol includes full
256-KiB job-envelope validation and other codec changes. No mixed-pin runtime
compatibility was tested. Select one coherent family and run each repository's
pin-convergence checks before a live proof; do not re-pin silently.

## Required WP2 live proof

Option A admits a content-free, actor-authorized RouteInvocation. Runner-target
route declarations live on `offchain/` handlers and lower to `__cowboy/routes`
at deploy, bound to the built bundle hash. The implementation still needs:

- Immutable actor code/runtime binding, actor authorization, stable invocation
  ID, runner assignment, secret references, budget, cancellation, and replay
  semantics bound to the same job. Private inputs must stay off consensus.
- An off-consensus runtime for the authored handler and its SDK calls. A
  container or harness process launching successfully proves only execution,
  not this entire contract.
- An identified actor debit/reservation and metered settlement path with
  content-free receipts; output and error serialization must be inspected too.

The WP2 gate must run a synthetic actor through the real route. Record compatible
artifact pins, route binding, invocation/job IDs, finalized assigned runner,
verify-only then read-secret authorization, actor debit/usage, and settlement
evidence. Query the actual committed job/receipt payloads for the synthetic
plaintext sentinel. Exercise forged wake, wrong actor/network, unassigned
runner, budget exhaustion, retry, and interruption. Measure representative
ROOM_KEY/ROOM_SEAT/wake writes and the maximum intended actor allowlist against
the transaction limits (100k cycles/20k cells/512 accesses in current dashboard
secret writes; access counts charged rows, not secret-read frequency).

Phase B follows the new Sign support: sign an encrypted reply without exposing
the seat key to Python, verify it independently, and retry the wake/append
without creating a second reply identity or changing the sealed bytes.

No deployment, funding, synthetic job submission, secret release, or live
content-leak test was performed in this source-inspection slice. A runnable
end-to-end harness depends on the missing admission/execution contract; this
report deliberately does not replace it with mocks or a source-scan pass flag.

## Admission options sent to the coordinator

Both options require actor-authorized admission. At
`node/execution/src/runner/dispatcher.rs::handle_job_submit`, `decode_and_admit`
stamps the job submitter from `tx.from`. CBSS's `validate_job_assignment` then
requires that submitter to equal the actor requesting release, as well as
requiring the runner to be assigned. `callback.actor` is deliberately not an
authority source. An ordinary gateway- or runner-signed JobSubmit therefore
cannot become an actor job by putting the actor address in its callback.

**A: admission relayed by gateway.** A new generic route-invocation descriptor
and node admission path verify actor authorization, immutable handler/runtime
binding, invocation replay, secret references, and budget before creating an
actor-owned job. Gateway transports the request without gaining spending
authority. Runner executes the private handler after assignment. Protocol,
node, runner, and gateway change; CBSS keeps the actor/assignment check and
later gains Sign entitlement support.

**B: admission relayed by runner.** Gateway forwards as today. A new runner
route endpoint relays the same actor-authorized request to node and waits for
assignment before release or execution. This moves admission initiation and
adds pre-admission DoS handling. It does not eliminate the protocol/node
authorization changes. The naive variant where a runner submits an ordinary
job under its own identity is incompatible with the existing CBSS check.

A was chosen by Chad; B is retained here as the rejected alternative and its
security analysis. Neither option is implemented. Existing
dispatcher `escrow_job_payment` and `escrow_container_compute` are candidate
payment mechanisms after actor attribution is established. Their u64 balance
and price representation must be explicitly reconciled with the u128-wei
invocation grant; no narrowing cast or unverified unit conversion is implied.

The chain will still expose authorization/billing metadata such as actor and
job IDs, runner assignment, artifact references, secret key hashes, limits,
usage/status/timing, and commitments. Private content need not be among it.
Use encrypted/opaque input references and content-free results; a hash of
guessable plaintext is not a privacy guarantee. Audit Container result/error
serialization before asserting this property on existing usage rails.

The unchanged CBSS actor/assignment check is a constraint of Option A. It does
not imply no future CBSS edits: `validate_actor_manifest` currently recognizes
only `secrets.read` and `secrets.verify`. The later strict Sign work must add
`secrets.sign` recognition while preserving assignment, actor identity, ACL,
and purpose separation. That is separate from route admission.
