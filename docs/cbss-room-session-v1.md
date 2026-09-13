# CBSS room-session release v1

Status: **proposal for Claude review; no endpoint or key-release implementation**.
This is stage 2 of [room key delivery](room-key-delivery-design.md). It follows
Chad's decisions relayed in dashboard-impl #1799: owner-delegated access authority,
standing approval of runtime identity keys, and initial CBSS setup off the serving
path. None of this authorizes live secret creation, deployment, or transactions.

## What this adds

A trusted actor, builder, or door host can obtain one exact secret version for one
short session without submitting a job or asking a browser to stay open. A new
CBSS proxy endpoint verifies a standing wallet grant, fresh off-chain access
statement, and proof of possession by the approved runtime. Each proxy returns
its threshold partial sealed to that session's ephemeral X25519 key. The runtime
verifies and combines partials, unwraps the secret in memory, and uses it through
a purpose-specific host interface. The model and ordinary conversation journal
never receive the key.

The new route is `POST /v1/room-session/releases`, with
`Content-Type: application/cbor`. It has its own request and response types.
There is **no job_id field**, optional legacy body, fabricated assignment, chain
receipt, or call to a chain-admitted release route. Existing job release and CIP-7
recovery keep their existing authorization checks. This route remains disabled
until its dedicated verifier, storage fencing, and conformance tests exist.

Acquire, renew, wake, read, sign, and reply use no consensus writes. Setup may
register the secret, committee, and ordinary CBSS policy once. Membership and
standing grants remain signed off-chain objects. A proxy uses local verified
snapshots and a supplied, independently verified finalized-proof bundle; it
never performs a chain RPC synchronously on this request path. A background
courier refreshes those snapshots. Missing or stale authority denies new release.

## Trust and capability boundaries

The runtime identity is a distinct Ed25519 host key explicitly approved in the
standing grant. It is not a URL, process ID, actor's self-reported address,
provider webhook signature, seat signing seed, or a key returned by the access
authority. The runtime alone chooses and signs the ephemeral recipient key.
An access-authority compromise can authorize continued release to an already
approved runtime or delay revocation. It cannot change the grant's runtime,
recipient, purpose, asset, room, or controller signatures. An approved runtime
that is compromised can copy released secrets; a lease is not cryptographic
revocation of a copied key or of a recovered IBE partial.

All three authorities countersign the same grant body, under distinct domains:

| Slot | Independently resolved wallet | Responsibility |
| --- | --- | --- |
| room | Current room owner | Membership, room scope and allowed host |
| seat | Current seat controller, or builder wallet | Host acting for this seat |
| asset | CBSS secret account owner | Release of this exact secret version |

They may be the same wallet, but all three slots must verify. An actor's current
controller comes from authenticated Cowboy state, not from an actor-provided
certificate. A human/builder identity requires its existing wallet signature.
A delegated membership admin cannot sign these wallet grants or renew identities.
For provider credentials the asset owner must also have completed the provider's
account authorization during setup; knowing a room key proves none of that.

One grant covers one purpose and one secret version. Reusing the same CBSS secret
coordinates across different purposes or rooms is forbidden during provisioning;
keep separate secrets even when their plaintext happens to match. Purpose codes:

| Code | Secret and permitted host interface | Required current authority |
| --- | --- | --- |
| 0 | 32-byte room generation secret; decrypt/encrypt the exact generation | Room membership read; write additionally before append |
| 1 | 32-byte Ed25519 seat seed; sign validated Cowchat request/record domains only | Seat controller, current identity and write membership |
| 2 | 32-byte subscription wake HMAC key; verify only | Subscription owner/revision and enrolled target seat |
| 3 | Provider credential bundle; configured provider operations only | Door controller, binding revision and provider-account authorization |

Code 2 does not grant room read or paid-turn authority. Code 3 does not grant room
read, signing, or broad provider-account access. Additional grants are required
for the bridge's other functions. Bindings use opaque public identifiers, never
provider tokens, phone numbers, chat text, or callback URLs. All host interfaces
must also authorize the actual operation: obtaining a key is not a billing
reservation, invocation grant, or permission to execute a model turn.

## Encoding and domains

`u` is an unsigned 64-bit integer; `bN` is exactly N bytes; `t` is UTF-8 text;
arrays below have exactly the listed length and order. Booleans are allowed only for the AuthorityState revoked field. No maps, tags, floats,
negative integers, indefinite lengths, unknown fields, or trailing bytes.
Use RFC 8949 deterministic CBOR: shortest lengths/integers and exact re-encoding.
Text is bounded to 256 bytes and must match the canonical enrolled identifier
byte-for-byte; no normalization in the verifier. Room is the canonical lowercase
hyphenated UUID text, not its display name. Seat is the enrolled canonical seat
identifier. Certificate IDs are the existing Cowchat SHA-256 certificate IDs.
Every time is integer Unix **seconds**. Every generation is nonzero.

For ASCII domain `d` and bytes `x`, define:

```
frame(d,x) = u16be(len(d)) || d || u32be(len(x)) || x
H(d,x)     = SHA-256(frame(d,x))
```

Lengths must fit their fields; never truncate. A byte-string reference below
contains the exact canonical encoded object, not a decoded/re-encoded JSON form.
All new signature domains start `cowchat/cbss-room-session/v1/`. Ed25519 signatures
are strict RFC 8032 signatures over `frame(domain, canonical_body)`, with no
prehash. Reject noncanonical/small-order verification keys and noncanonical
signatures. Wallet signatures use the existing Cowchat wallet convention:
Keccak-256 of that frame, compact secp256k1 r||s plus recovery byte, exactly 65
bytes, low-S, recovery ID 0..3, and the recovered uncompressed-key Keccak address.
There is no personal-sign prefix or Ethereum message wrapper.

Limits before allocation: HTTP request 64 KiB, reply 4 KiB, embedded grant 4 KiB,
embedded request 4 KiB, access body 4 KiB, signatures as specified; depth 8 and
512 CBOR items per object. Oversize/truncated/noncanonical inputs fail before
signature checks, storage, or partial computation.

## Stable asset identity and provisioning

```
Asset = [1, chain:u, account:b20, key_hash:b32, version:u,
         wrap_epoch:u, master_public_key:b96]
asset_id = H("cowchat/cbss-room-session/v1/asset", CBOR(Asset))
```

The master public key is the verified release key's compressed BLS12-381 G2
`vss_commitments_g2[0]`, not caller-selected material. Account/key_hash/version
use the existing CBSS `SecretId` and metadata. Asset is an authorization envelope;
**it does not replace CBSS's stable IBE identity**. Use existing
`IdentityInput { chain_id, account, key_hash, version, wrap_epoch, mpk_g2 }`, its
172-byte base AAD, and `compute_identity`. Session IDs, nonces, committee serve
epochs and recipients must never enter this stable IBE identity. Otherwise a
fresh session would be unable to decrypt the already wrapped secret.

The wrapped-DEK scheme and payload encryption remain the existing release-pinned
CBSS account-secret format. Retain its full ciphertext authentication (including
its ephemeral IBE component); this spec's HPKE AAD below is an additional outer
layer. No raw threshold partial is a signature over a room message.

Provision a dedicated secret and release key/committee configured to serve this
room-session policy. A wallet-signed grant alone must never reinterpret arbitrary
existing CBSS secrets as room-session secrets. The local proxy provisioning
registry pins `asset_id`, the grant identity, and this policy version, based on
verified owner setup. Legacy release dispatch must reject these dedicated assets;
the new dispatch must reject all unregistered assets. Use separate dedicated
committee configuration for v1 if a shared dispatcher cannot enforce both
exclusions. This is a proxy policy fence, not a new per-message consensus feature.
Existing job authorization is neither removed nor made optional.

The verifier independently resolves secret metadata, release-key scope,
committee epoch, member indexes, proxy authentication keys, threshold, commitments,
controller proof and chain identity from its verified snapshot. A request does
not supply replacement trust roots. Pin the source protocol release and proof
verifier in configuration; reject mismatched/unsupported proofs. Resharing may
change serving committee/commitments only through verified snapshot advancement,
with the asset's master key and wrap epoch unchanged. A new master key or wrapped
secret version requires a new Asset and grant.

## Standing grant

```
Grant = [1, grant_id:b32, grant_generation:u, Asset,
         room:t, seat:t, identity_cert:b32,
         room_owner:b20, seat_controller:b20,
         auth_generation_min:u, auth_generation_max:u,
         key_generation:u, transport_generation:u,
         purpose:u, resource_id:b32, resource_revision:u,
         runtime_ed25519:b32, access_authority_ed25519:b32,
         not_before:u, expires_at:u_or_null]
SignedGrant = [grant_cbor:bstr, room_sig:b65, seat_sig:b65, asset_sig:b65]
```

Signature suffixes are `grant/room`, `grant/seat`, `grant/asset` respectively.
`grant_digest = H("cowchat/cbss-room-session/v1/grant", grant_cbor)`.
Asset account is the asset signer; the other two wallets must match independently
verified ownership. `grant_id` is random nonzero 32 bytes allocated at setup and
permanent across revisions; a new body requires a strictly greater generation.
Same ID/generation with a different digest is an equivocation error.

`expires_at = null` means until revoked, not immunity to controller, certificate,
membership, runtime or secret-policy changes. Bounds are explicit and inclusive;
minimum must not exceed maximum. Key and transport generations are exact. V1 new acquisitions target the current
room key generation and must also satisfy membership `from_gen`; this first
release surface does not yet distribute earlier history generations. Reading
older encrypted history after a rekey requires an explicit historical-generation
policy extension, never widening the current-key grant implicitly. For
purposes 0/1, resource_id is zero bytes and resource_revision is 0. For purpose 2
it identifies the installed subscription/revision; for 3 the configured door and
binding revision. Those two purposes require nonzero identifiers/revisions.
No implicit rotation wildcard, automatic broadening, or authority-key replacement
exists in v1. A new room-key version/certificate or changed bounds require fresh
wallet approval. Actor certificates without expiry remain subject to controller
changes and membership revocation. Builder certificate expiry can stop unattended
renewal; automatic identity renewal is a separate delegation design, not hidden
inside this grant.

## Runtime request and fresh access statement

```
Request = [1, grant_digest:b32, grant_id:b32, grant_generation:u,
           asset_id:b32, room:t, seat:t, identity_cert:b32,
           auth_generation:u, key_generation:u, transport_generation:u,
           purpose:u, resource_id:b32, resource_revision:u,
           session_id:b32, operation_id:b32, nonce:b32,
           recipient_x25519:b32, issued_at:u, expires_at:u]
request_digest = H("cowchat/cbss-room-session/v1/request", request_cbor)
```

The runtime signs under suffix `request/runtime`. Session ID and nonce are
independent unpredictable nonzero values. Operation ID is the trusted host's
stable logical operation reference; it contains no prompt or provider payload.
Each acquisition uses a new nonce and ephemeral X25519 key. Requests must match
every grant scope field and its generation bounds, with `issued_at <= now <
expires_at`, `expires_at - issued_at <= 30` seconds, and no future-skew allowance.
Expiry must not exceed any applicable certificate, grant, invocation or host-run
lease. Hosts synchronize clocks; skew fails closed rather than extending leases.
Long builder work renews these short acquisitions while its separately authorized
run remains alive; this v1 does not extend the current actor wake's 30-second bound.

The runtime obtains the following signed body from the *grant-named* access
authority using the already signed request. That authority verifies the runtime
signature and independently checks current membership, controller, purpose,
resource revision, applicable invocation, and scope before signing:

```
Access = [1, request_digest:b32, grant_digest:b32,
          checkpoint_digest:b32, authority_state_digest:b32,
          controller_generation:u, authority_sequence:u, issued_at:u, expires_at:u]
```

Signature suffix `access`. Checkpoint digest refers to the independently verified
public authority snapshot and its proof bundle, not an authority-provided list of
new keys. For actor/door seats, controller generation is the authenticated
`__cowchat/control/v1.authorization_generation`, with the independently verified
controller and certificate commitment as required by actor enrollment. For a
wallet builder/owner it is the current wallet-signed identity generation; this
does not invent an on-chain account counter. A controller mismatch invalidates a
grant even if a faulty control update failed to increase the generation. Authority sequence is monotonic per `(chain, grant_id)`; it remains
constant for unchanged authority state and increases for revocation/scope changes.
Repeated attestations for the same state may have the same sequence. Same sequence
with a different grant or authority-state digest is equivocation. The state digest
covers the canonical public authority projection below, excluding proof observation
time and finalized-head advancement, so courier refresh alone is not equivocation:

```
AuthorityState = [1, chain:u, room:t, seat:t, grant_id:b32,
                  grant_digest:b32, controller:b20, controller_generation:u,
                  identity_cert:b32, auth_generation:u, key_generation:u,
                  transport_generation:u, membership_rights:u,
                  resource_id:b32, resource_revision:u, revoked:bool]
```

Membership rights are bits 1=read, 2=write, 4=manage, with no unknown bits.
`authority_state_digest = H("cowchat/cbss-room-session/v1/authority-state",
CBOR(AuthorityState))`. The proxy constructs this projection from its verified
local authority store. The access authority must agree with it; its signature
cannot install a new projection or undo a locally observed revocation. Checkpoint
digest identifies the proof bundle which established that projection and may
change without an authority-sequence increment.

The access body binds the exact request and thus its recipient and every scope
field. It may only narrow lifetime: `issued_at <= now < expires_at <=
request.expires_at`, and statement age plus the independently verified room-proof
age must each be <=60 seconds. For chain proofs, age is measured from the authenticated finalized
target timestamp, never from the time the courier happened to return old bytes.
Retain the existing finalized-proof policy of at most five seconds of future
skew. Access/request timestamps themselves get no future-skew extension. Convert
existing millisecond proof/certificate timestamps with checked arithmetic,
rounding expiry down; never confuse seconds and milliseconds. Local off-chain
room state must agree with the fresh grant-named access statement. If current finalized/controller authority cannot be
verified, decline new releases. Existing room passive-authority serving semantics
remain separate; an outage does not rewrite membership.

The wire request is exactly:

```
[1, SignedGrant, request_cbor:bstr, runtime_signature:b64,
    access_cbor:bstr, access_signature:b64]
```

The proof bundle is supplied through the separately authenticated background
snapshot importer, not this public HTTP body. A cache miss returns a fixed retry
error without triggering RPC, refreshing identity, or submitting a job. No bearer
token, wake header, or successful legacy job request substitutes for these checks.
A runtime-signed random nonce provides proof of possession and replay binding;
there is no additional proxy-issued challenge round trip.

## Proxy authorization, durable replay, and response

Before generating a partial, each proxy validates canonical encoding, all
signatures, registered-asset policy, grant scope, local authority snapshot and
freshness, recipient X25519 validity, and every current generation/expiry. It
checks that its own index/key belongs to the independently resolved committee.
Reject all-zero/low-order X25519 keys or an all-zero shared secret.

Maintain a durable nonregressing floor per `(chain, room, seat, grant_id)` for
grant generation/digest, controller revision, room authorization generation,
key/transport generations, access authority sequence/state digest, and revocation.
A revoked grant ID is terminal; reauthorization uses a new grant ID. A generation
jump is not established by the request itself: only an authenticated snapshot
update can advance a floor. Unknown grant/runtime/policy, stale or conflicting
snapshot, or any regression denies release. Verify current state and commit the
replay result in the same local authority-store transaction/fence so revocation
cannot interleave with a newly authorized response.

Replay key is `(asset_id, grant_id, grant_generation, runtime_key, nonce)`.
Reserve it durably with request digest before cryptographic work, persist the
sealed response before returning, and serialize concurrent reservations. Exact
retry rechecks current authority and expiry, then returns the stored bytes;
changed request digest is a conflict. A crash after reservation but before stored
response may recompute under the same recipient only after full reauthorization.
A crash after persistence returns the same envelope. A stale process must not
publish its computed result after authority changes; use a generation fence.
The final fence check immediately precedes response publication; network bytes
already emitted before a revocation cannot be recalled. Expired retries do not
replay a previously valid secret release. Keep replay entries at least through
expiry plus 60 seconds; wall-clock regression or floor-store loss pauses serving.
Restore requires a verified nonregression checkpoint before enabling the endpoint.

```
Header = [1, request_digest:b32, asset_id:b32, grant_digest:b32,
          session_id:b32, recipient_hash:b32,
          committee_epoch:u, committee_digest:b32,
          proxy_id:b32, proxy_index:u, expires_at:u]
Committee = [1, chain:u, committee_epoch:u, threshold:u,
             [[proxy_id:b32, proxy_index:u, proxy_ed25519:b32], ...],
             [commitment_g2:b96, ...]]
```

Members are sorted by positive unique proxy index; IDs and keys must be unique.
Threshold is positive, no larger than member count; commitment count equals
threshold. `committee_digest = H("cowchat/cbss-room-session/v1/committee",
CBOR(Committee))`. Values come from the verified registry/snapshot. A dedicated
Ed25519 response-authentication key for each proxy is pinned there during setup;
a wire response cannot introduce its own key. This is distinct from the BLS
share, wallet key, and runtime key. `recipient_hash = H("cowchat/cbss-room-session/v1/recipient",
recipient_x25519)`; header expiry equals the access expiry.

Use RFC 9180 HPKE Base mode: DHKEM(X25519, HKDF-SHA256) 0x0020,
HKDF-SHA256 0x0001, AES-256-GCM 0x0002. Fresh encapsulation, one seal at sequence 0.
Info is the literal ASCII `cowchat/cbss-room-session/v1/partial-hpke`.
AAD is `frame("cowchat/cbss-room-session/v1/partial-aad", header_cbor)`.
Plaintext is the existing canonical 48-byte compressed G1 partial for the stable
Asset IBE identity. Ciphertext is therefore 64 bytes; encapsulated key is 32.

```
UnsignedResponse = [header_cbor:bstr, hpke_enc:b32, hpke_ciphertext:b64]
Response = [1, UnsignedResponse, proxy_signature:b64]
```

Proxy signs canonical UnsignedResponse under suffix `partial/response`, binding
both encapsulation and ciphertext in addition to the HPKE AAD scope. HTTP 200 is
only this response. Other responses have fixed content-free codes: malformed 400,
unauthorized 403, conflict 409, authority unavailable 503, rate limited 429.
No redirects; `Cache-Control: no-store` for every response. Bodies/errors/logs
never include tokens, plaintext, raw partials, request bodies or callback URLs.
Limits on concurrency/retained reservations are operator configured; they do not
consume a billing hold or create a chain receipt.

## Runtime recovery and lease use

The runtime freezes one verified committee snapshot for a collection attempt.
For every response, before unsealing, verify the proxy signature and exact expected
header including request, asset, grant, recipient, session, committee and expiry.
Reject duplicate indexes, mismatched commitments, unknown proxy keys, another
session or an expired response. After HPKE open, apply the existing strict G1
parse and VSS partial verification against the stable identity. Combine at least
the verified threshold of unique indexes from that one committee, then unwrap
with the existing CBSS AAD and authenticated ciphertext. Never combine merely
because the inner partials happen to verify for the same stable asset.

The host exposes a nonserializable, zeroizing purpose-specific handle bounded by
access expiry, and rechecks authority before key use, output signing and append.
Cancellation/expiry drops handles and intermediate shares; restart reacquires.
No key in environment variables, command arguments, model tools, ordinary event
journals, durable job outputs, debug/error strings or tracing. Persist ciphertext,
public grants/certificates, and bounded content-free release references only.
No fallback to a developer wallet, runner identity file, cached plaintext, or
unbounded room key. Byte-copy zeroization inside third-party/provider code is not
claimed; the host is within the explicit trust boundary.

Revocation refuses the next release and the next authorized append. Re-key future
room traffic on removal. Neither proxy refusal nor an expiring wrapper can erase
already decrypted history or stop a malicious runtime using a copied stable key.
Browser closure does not itself revoke an approved runtime; expiry of its actual
run, controller/identity/grant invalidation, or explicit revocation does.

## Vectors, implementation order, and acceptance

[Public golden vectors](fixtures/cbss-room-session-v1.json) pin deterministic CBOR,
framing, scope digests, runtime/access/proxy signing inputs and strict Ed25519
signatures. Run `python3 scripts/room_session_vectors.py --check`. The script uses
published RFC 8032 test seeds only and contains no live key, encryption, network,
CBSS release or runtime implementation. It also checks an independent CBOR
encoder and binding mutations. Wallet signing inputs are pinned; wallet and HPKE
ciphertext/BLS interoperability vectors must be added in their implementation
slices before those paths are enabled. This fixture does not claim an end-to-end
cryptographic release proof.

After spec approval: (1) pure canonical types/verifiers and cross-language crypto
vectors, (2) proxy policy/snapshot/replay store with legacy-negative tests, (3)
client sealed-partial recovery, (4) trusted runtime provider and fresh acquisition,
(5) local real-proxy threshold test and private wake integration. None implicitly
activates a service. Release pins and a separate provisioning review are required
before creating real secrets or enabling a public route.

Required negative cases: altered room/seat/cert/purpose/asset/version/generation,
unknown host or proxy key, redirected recipient, stale/future proof, controller
change, revoked grant, wrong policy, mixed session/committee/index, malformed CBOR,
wrong domain/signature, low-order keys, altered wrapped ciphertext, expired exact
retry, restore regression, authority update racing a release, and legacy requests
with missing/invented job assignment. Prove independent grants cannot substitute
for each other or expose provider/HMAC assets through room-read tools.

Required local integration: valid scoped release without any job or chain-write
caller; forged wake releases no keys; browser-closed short renewal; restart
reacquisition; removal blocks next acquire/append; crash-after-append retry keeps
one reply; plaintext/key canaries absent from all durable stores and traces.
Metering/funding/settlement is a separate contract and never added to this route.

Source inventory used for this draft: Cowchat `a2be38b24e2b9211a6582a16325a54b24b01837d`
(`certificates.rs`, `signatures.rs`, `keys.rs`, stage-1 design/envelopes); CBSS
`fd5bb4691989df5fa4bd50d1c4a12756b40c1759` (`types.rs`, `identity.rs`, `combiner.rs`,
`cip7_delivery.rs`, `chain_authorizer.rs`). Existing stable account-secret identity
was checked in source; it never contained a job ID. Only the legacy authorization
request does. No source or consensus implementation was changed for this draft.
