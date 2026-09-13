# Actor enrollment proof boundary

Actor certificate matching, finalized-control proof fetching, and
`POST /rooms/{room_id}/actors` are implemented. The endpoint accepts the same four
base64 certificate/signature fields as owner enrollment, plus signed-request
headers under the actor's delegated key. It verifies room-owner membership and
request possession before fetching any proof. The final transaction independently
rechecks room state, proof freshness, certificates, and request signature, consumes
the nonce, installs credentials, and advances the local proof rollback floor.
An exact retry with a fresh request nonce preserves the existing installation.

The matcher checks two separate authorities: the actor controller signs the actor's
identity certificate, and the room owner signs its membership certificate. The
identity certificate must match the commitment and authorization generation in
the actor's authenticated control record. Room membership and encryption-key
generations are separate. Membership does not authorize paid invocation.

`actor_proof::ActorProofAuthority` fetches `POST /proof/finalized-state` from a
service-configured RPC, requests exactly the actor and `__cowchat/control/v1`, and
calls the existing `cowboy-protocol-light-client` verifier. The proof must contain
the exact key under the actor's address. Its canonical CBOR value has three fields:
`controller` (20-byte byte string), `certificate_commitment` (32-byte byte string),
and `authorization_generation` (unsigned integer). This is newly specified actor
application data; its deployment has not been established.

The release build may supply `COWCHAT_RELEASE_CHECKPOINT_PATH` and
`COWCHAT_RELEASE_CHECKPOINT_SHA256` together. The build embeds the checkpoint bytes
and expected digest into the binary. The loader validates the digest and canonical
checkpoint. No runtime path or HTTP-provided checkpoint can change that anchor.
Without these build inputs, the authority constructor refuses enrollment. No
production network checkpoint is included in this slice. `serve --http ...
--actor-proof-rpc <origin>` enables the endpoint's proof authority in such a build;
without that runtime option the actor enrollment endpoint returns unavailable.

Before shipping: embed independently verified production checkpoint bytes and
their digest at build time, configure the intended proof RPC, and verify that the
resulting proof identifies the intended chain instance and a fresh finalized tip.

The configured RPC is a courier, not the trust anchor. Requests have a ten-second
timeout, refuse redirects, and bound the response by the protocol proof limit.
The authenticated target timestamp must be at most sixty seconds old and no more
than five seconds in the future. This is a bounded freshness policy, not an
assertion that no control update can occur after the read. Returned verified
control values have private fields. The enrollment transaction rechecks freshness
and room generations and rejects proof height regression, a conflicting hash/root
at the same height, changed chain instance, actor authorization regression, or
a changed certificate commitment without advancing actor authorization. The
height/hash/root/generation/commitment floor is local SQLite state. A newly proven
higher actor authorization generation invalidates older actor credentials across
rooms on the same chain. `Store::ingest_actor_control` can also consume a fresh
`VerifiedActorControl` for an already tracked actor without installing any
credential. It uses the same rollback-floor checks as enrollment. In one local
Immediate transaction it advances that floor, removes superseded actor
credentials across rooms, and fails their subscriptions while incrementing the
local revision to fence in-flight webhook responses. Queued pointer bytes remain
available for an explicitly authorized future repair. Owner credentials and actors
on other chains are unaffected. A failed transaction changes none of these states.

A server configured with `--actor-proof-rpc` and its release-pinned checkpoint
now starts a background refresh worker immediately. It scans tracked actors that
still have credentials, in pages of 32 with at most four proof requests in flight,
and waits 15 seconds after each completed sweep. The guard cancels the sweep and
its child requests when the server task ends. Each request uses the existing
10-second timeout and bounded finalized-proof response; there is no message-path
RPC and no new HTTP proof-ingestion endpoint or consensus write.

Freshness has two tiers. Enrollment/re-enrollment and a new subscription binding
(or repair onto a replacement certificate) require a verified control proof no
older than 60 seconds, allowing at most 5 seconds of future clock skew. Binding
checks consult only the cached proof timestamp, generation and controller in the
local transaction. An exact subscription retry is not a new binding. Existing
reads, appends, replies and wakes continue on last-ingested verified authority,
subject to current membership and certificate expiry. A courier outage does not
by itself delete credentials or stop steady-state flow.

The healthy-courier target is a refresh every 15 seconds plus sweep time. A sweep
of N actors makes N requests at concurrency four; network time can approach
`ceil(N/4) * 10 seconds`, plus local proof verification and database work. This is
not a hard revocation deadline: a courier may return a proof up to 60 seconds old,
and outages, withheld proofs or overload can delay revocation indefinitely until
a newer valid proof arrives (or a separately set credential expiry denies access).
A successfully ingested newer proof applies locally and atomically immediately.

Only a verified QMDB absence claim for the exact actor/control key revokes via
delete. Missing claims, malformed proofs, fetch errors and timeouts leave state
unchanged. Authenticated absence advances the local height/root floor, removes
all of that actor's credentials on the chain, and fails/fences its subscriptions.
The tombstone retains the last known authorization generation: restoration needs
a strictly higher generation and a separate enrollment, never automatic revival.
Present proofs also invalidate a credential whose enrolled controller differs
from the proven controller, even if the numeric generation did not change.
Existing databases gain local proof timestamp, controller and absence columns;
legacy actor credentials without a recorded controller must re-enroll after the
first successful refresh. Owner credentials remain independent.

History and initial subscription backfill exclude records from key generations
before membership's `from_gen`. History advances its cursor past excluded records
so a bounded page can reach the authorized range. No room key is stored here;
key delivery must also enforce that floor when it is connected.

```sh
cargo test --offline --locked -p cowchat-crypto --test actor_membership
cargo test --offline --locked -p cowchat-server --lib actor_control_fetch
cargo test --offline --locked -p cowchat-server --lib actor_http_enrollment
cargo test --offline --locked -p cowchat-server --lib passive_actor_control
```

The second test fetches an actual threshold-signed finality and QMDB inclusion
proof over local HTTP and rejects the wrong actor, altered proof, stale/future
timestamp, and malformed control value. Its chain and checkpoint are synthetic
test fixtures. It does not prove Canyon configuration, a live deployed actor,
secret acquisition, or runtime execution. The enrollment HTTP test uses distinct
owner and actor keys and the real proof verifier, injects installation failure to
check complete rollback, exercises exact retry and preflight/install state races,
rejects an older proof than the local floor, and appends/reads/decrypts an
authenticated actor record while excluding an older key generation. The actor
record is written by the test client; wake-triggered actor execution through a
generic gateway is separately covered by the [local fixture proof](local-m-echo.md).
Proof fetching belongs to enrollment and the background refresh worker; store
ingestion accepts only privately constructed verified proof values. Room append, history, and wake delivery
continue against local service state. Ingestion tests use real synthetic
threshold-finality/QMDB verification with provisioned store fixtures, not a live
chain. They cover cross-room revocation, isolation of owners/other chains,
rollback on an injected deletion failure, stale in-flight wake outcomes, repeated
proof ingestion, persisted rollback floors, and stale/conflicting proof rejection.

The refresh tests exercise actual local HTTP courier failures, invalid proof bytes,
and a real threshold-signed QMDB absence proof: failures preserve state, while
absence fails the subscription and removes the actor from subsequent scans. Tests
also cover same-generation controller mismatch, authority-changing cached-proof
freshness versus steady-state wake availability, and cancellation on server guard
drop. These remain local synthetic-chain tests, not a deployed Canyon watcher.
