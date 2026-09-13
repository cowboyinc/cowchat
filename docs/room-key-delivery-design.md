# Room key delivery without per-message consensus

Draft for Claude's design ruling, 2026-09-13. No key-release implementation or
production activation is authorized by this document alone. This supersedes the
old implementation plan's assumption that every room wake creates a
`RouteInvocation` job; that work remains frozen.

Keep user access grants durable until revoked, and make execution credentials
short-lived and automatically renewable underneath them. A closed browser must
not interrupt an authorized builder or prevent an actor waking months later.
Renewing credentials does not ask the person to approve the same access again.
Reading a room, being notified about it, and starting computation remain separate
permissions.

## What the current code actually provides

The room service already verifies controller-backed actor identity, room-owner
membership, signed requests and records, and passive revocation. Its metadata
and authenticated ciphertext APIs do not distribute private keys. The harness
wake handler has a `CowchatWakeRuntime` boundary and a transient
`CowchatWakeLease`; it has no production key provider. The successful local demo
uses explicitly provisioned fixture keys.

`cowchat-crypto::keys` implements X25519 HPKE room-key wrapping and unwrapping.
That handles transport to a known encryption key. It does not establish who owns
that key, who may obtain a wrapping, or how the private key survives a restart.
Those checks and lifecycle rules are the work proposed here. Human/builder
identity certificates currently require the wallet signature; the delegated
Ed25519 admin signer is a membership facility, not a general identity-renewal
issuer (`crates/cowchat-crypto/src/certificates.rs`). Automatic identity-key
rotation under an until-revoked grant therefore needs an explicitly verified
renewal-delegation path. It cannot be implemented by allowing the harness to
forge a wallet signature or retaining the wallet private key.

The inspected CBSS release API is job-bound: `ReleaseRequestBody` includes a
`job_id`, and cbssd verifies both the actor as job submitter and the assigned
runner before releasing a share. CIP-7 subscriber recovery also consumes
chain-accepted partial-delivery rows. Neither is a ready-made off-consensus room
session release API. We must not invent a job ID, bypass the assignment check,
or silently reactivate the frozen route-admission work.

Code evidence at the inspected local heads:

| Boundary | Source |
|---|---|
| HPKE wraps | `cowchat` 4694d55, `crates/cowchat-crypto/src/keys.rs`, `wrap_room_key` / `unwrap_room_key` |
| Transient harness keys | `cowboy-harness` 8a53482, `crates/harness-serve/src/cowchat_wake.rs:45`, `CowchatWakeLease` / `CowchatWakeRuntime` |
| Required CBSS job binding | `cbss` fd5bb46, `crates/cbss-client/src/types.rs:179`; `crates/cbssd/src/chain_authorizer.rs:137` and `:180` |
| CIP-7 recovery boundary | same CBSS head, `crates/cbss-client/src/cip7_delivery.rs` |

## Proposed shape

Use the existing HPKE path for human and builder recipients. For actor and door
recipients, add a **separate, explicitly authorized room-session release mode**
to CBSS, subject to this design ruling. Reuse the threshold and envelope crypto;
define a new authorization request instead of weakening the job-release request.
There are no consensus writes in acquire, renew, read, sign, wake, or reply.

Initial storage/provisioning is a separate unresolved boundary. This design does
not authorize creating a new on-chain CBSS secret, release key, stream, or policy.
It can operate against an already provisioned secret and committee once the new
release authorization exists. If room onboarding requires new chain state, that
must be resolved against Chad's instruction before implementing that step.

A standing grant is signed by the authority that can delegate each asset. The
room owner grants room-key access and room membership. The actor controller
grants access to that actor's signing key and designates its execution identity.
Neither authority can substitute for the other. The common case of one wallet
owning both should use the same checks, not a special shortcut.

The grant specifies chain, room, seat and certificate, membership/key-generation
bounds, exact secret IDs and versions or a deliberately bounded rotation policy,
allowed operations, and the approved runtime identity. It names the authority
that may issue replacement runtime credentials within those bounds. An explicit
owner/controller action is needed to broaden the bounds or replace that renewal
authority. A room service, gateway, message sender, or new process cannot nominate
itself as the recipient.

The standing grant has no user-facing expiry. Individual releases are constrained
by an expiring, recipient-bound runtime session and the current revocation state.
Runtime identity comes from a key authenticated by the standing grant and a
challenge signature binding an ephemeral X25519 recipient key. A URL, process ID,
actor address in an HTTP body, or unsigned gateway context is not an identity.

## Asset and purpose separation

| Asset | Authority | Authorized use |
|---|---|---|
| Room generation secret | room owner and current membership | read authorized generations; encrypt room output |
| Seat signing seed | actor controller, or wallet's builder delegation | sign this seat's validated request/record domains |
| Wake verification secret | subscription owner | verify notification HMAC only |
| External provider credential | door controller plus provider account authorization | that door's configured provider operation |

A release of one asset cannot authorize another. A wake HMAC is not permission to
obtain the room key, and possession of a room key is not permission to start a
paid turn. The actor/door signing seed stays inside the trusted execution host;
it is not exposed through a model tool, environment substitution, or user code
that was authorized only to read room text. This is host trust, not proof that an
untrusted host cannot copy a key.

Current `CowchatWakeLease` returns zeroizing seed and room-secret buffers to the
Rust handler. Its first production provider should preserve that narrow boundary.
A later opaque signer interface can reduce copies, but current CBSS IBE partials
are not message-level threshold signatures and must not be described that way.

## Delivery sequence

1. **Provision and enroll.** Generate a room generation secret in the owner's
   client. Generate the seat identity material under its controller's authority.
   Publish only public certificates, commitments, references and ciphertext to
   the dashboard and room service. Establish the standing grant once. Existing
   secret provisioning must be identified explicitly; there is no implicit
   permission to write consensus state here.
2. **Authenticate the execution host.** The host proves possession of its granted
   runtime identity key and binds a fresh ephemeral recipient key to its session,
   requested room/seat, operation and nonce. The host stores no room secret in
   `runner_key.json`; its process identity is not a general grant to actor secrets.
3. **Authorize a release.** CBSS proxies independently verify the same signed
   standing grant, current actor/controller proof and room membership, recipient
   identity, purpose and requested generation. A valid webhook is optional input
   for correlation and cannot replace those checks. Release is an authority-
   changing operation: require the existing 60-second proof freshness policy;
   failure denies this new release without rewriting room membership.
4. **Deliver encrypted shares/material.** Bind the release response to the exact
   chain, room, seat, certificate, key generation, secret version, purpose,
   grant generation, recipient-key hash, session ID and expiry. Use a distinct
   canonical signing/AAD domain from legacy job release. Existing room-key HPKE
   info binds room and generation only, so it needs an authenticated outer
   envelope for the additional scope; bare HPKE ciphertext is not authorization.
   A proxy never accepts a caller-supplied committee or checkpoint.
5. **Use and erase.** The trusted recipient verifies and combines the release,
   unwraps only the requested generation, and supplies transient buffers to the
   handler. The handler independently authenticates the trigger and invocation
   permission before model execution. Current room wakes are bounded to 30
   seconds. Their key lease must end no later than that invocation's lifetime.
6. **Renew while authorized.** A longer builder operation may reacquire short
   runtime credentials under the standing grant without the browser. It must
   check revocation before further reads or output. This does not extend the
   current 30-second actor wake handler into an hour-long job; long computation
   and its resumption belong to the separately specified runtime work.

The new CBSS request should be a separate versioned type with no `job_id` field.
All signed fields above are mandatory; changing the recipient, purpose, room or
version invalidates the request. Final wire tags and domains are a next-step
artifact after design approval, shared by client and proxies with golden vectors.
Do not accept both the old job interpretation and the new session interpretation
for the same request bytes.

## Revocation and persistence

Key-release authorization needs current, authenticated room access state as well
as the actor proof. The room service currently holds local membership generations;
there is not yet a proxy-verifiable export of that state. Define an owner-delegated
access authority whose signed responses bind the grant generation and session
recipient, and make proxies retain a rollback floor. That authority must be named
by the standing grant. This is a new explicit trust boundary: if it can approve
release only to preapproved runtime keys, compromising it cannot select an
attacker's new recipient, but it can delay revocation or continue authorizing an
already approved host. Do not call this permissionless or independently trustless.

The existing periodic chain refresh is a healthy-courier target of 15 seconds
plus sweep time. During an outage or proof withholding, steady room operations
retain last verified state; this has no hard revocation-latency bound beyond an
applicable credential expiry. **New key releases fail on stale authority.** A
previous recipient may already have the key and the plaintext; a new release
policy cannot erase either. Membership removal must rekey future traffic to
exclude the removed recipient. Keep that cryptographic boundary distinct from a
service refusing reads and from expiring an execution credential.

Durable storage contains only encrypted room/seat key material, signed grants,
public certificates and content-free release/revocation metadata. Room service
and dashboard plaintext logs, process arguments, environment variables, exception
messages, crash reports, run journals and model tool traces must not contain
private keys. The authorized runtime sees room plaintext; its transient room-mode
journaling work is still required. No privacy claim follows merely from zeroizing
one buffer.

Restart recovery reacquires keys from an authorized provider. It must not silently
load a room secret from a runner identity file or fall back to a developer key.
Local revocation and generation floors must survive restore; a stale backup must
not reactivate an old grant. Define that recovery check before enabling a
production release authority.

## Implementation cuts and acceptance

First implement the human/builder key-envelope service using existing HPKE and
signed recipient certificates. Wallet-issued builder bootstrap is now covered
by the [builder enrollment service](builder-enrollment.md). The renewal-delegation
verifier remains an explicit dependency. Identify the browser and builder recipient-private-key
recovery mechanism before claiming restart support. Wrap publication must require
the current owner or independently authorized key administrator; possessing a
readable member certificate does not authorize choosing the room's key. The
service must accept only authenticated, scoped wraps;
the service itself never unwraps. Test wrong chain/room/seat/certificate/generation,
recipient substitution, replay, removed members, and a signer trying to supply a
new encryption public key not in the recipient certificate.

Then specify and review the separate CBSS room-session authorization type and its
standing-grant/access-authority verification. This is required for autonomous
actor/door key delivery without a per-wake chain job. Cover purpose confusion,
unknown runtime recipients, stale proofs, controller changes, grant revocation,
committee mismatch, and mixed-session partial responses. The legacy job release
path must still reject a missing or invented job assignment.

Finally implement a production `CowchatWakeRuntime` key provider and repeat the
three-process proof with actual enrolled actor keys and a real release service:
forged wake releases no room/signing key; valid wake obtains only its seat's
material; restart reacquires it; revocation blocks the next release and append;
crash after reply still yields one authenticated visible reply. Prove browser-
closed renewal under an existing grant, and scan all persistence/logging sinks
for test key material and room plaintext. Synthetic fixture success alone does
not clear this gate. External doors use the same mechanism regardless of Telegram,
Slack, SMS, WhatsApp, or another provider.

## Ruling requested

Approve the staged direction above: HPKE recipient-envelope delivery first;
separate CBSS room-session authorization for actors/doors; durable until-revoked
user grants with automatic short runtime credentials; fresh authority for new
release and no consensus writes in these flows. Confirm the explicitly delegated
access-authority trust boundary before its wire types or secret-handling code.
Initial CBSS provisioning remains subject to the outstanding no-consensus scope
ruling. No frozen Track R work is resumed by this proposal.


## Review ruling (2026-09-13, room message 1656)

Claude approved stage 1 (scoped HPKE delivery for already authorized human/builder
recipients) for implementation. [The service contract](room-key-envelopes.md)
records the current implementation and its enrollment/renewal prerequisites.
Stage 2's separate CBSS session-release direction is approved, but its wire types
and secret-handling code remain gated on a concrete specification with golden
vectors reviewed by Claude and Chad's confirmation of the named access-authority
trust boundary and provisioning scope. Stage 3 follows those decisions. No new
user authorization or Track R activation is inferred from this ruling.


Claude's message 1660 approved the scoped-envelope service core (committed
b73173d) and authorized wallet-issued builder enrollment as the next bounded
cut. Delegated identity renewal stays out of scope pending the stage 2 ruling.
The builder HTTP enrollment and owner-to-builder delivery proof are implemented
for review; this does not complete automatic renewal or production key recovery.
