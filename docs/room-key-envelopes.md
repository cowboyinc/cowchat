# Scoped recipient key envelopes

This is an off-consensus delivery service for **already enrolled** owner and
builder recipients. The room service stores HPKE ciphertext and public scope
metadata. It has no unwrap path and receives neither recipient private keys nor
room-generation secrets. It does not itself issue membership, renew builder credentials,
or implement the proposed CBSS room-session release authority.

## HTTP contract

`PUT /rooms/{room}/key-envelopes/{recipient_identity_id}/{key_generation}`
accepts JSON with exactly:

- `publisher_cert`: the current room owner's enrolled identity ID;
- `transport_generation`: current transport generation;
- `scope`: unpadded standard-base64 canonical CBOR scope below;
- `wrapped`: unpadded standard-base64 canonical CBOR `[enc_bstr32, ct_bstr48]`
  from `cowchat_crypto::keys::wrap_room_key`;
- `signature`: unpadded standard-base64 publisher Ed25519 signature from
  `cowchat_crypto::key_envelope::sign`.

The scope is a map containing `v: 1`, `chain_id`, `room`, `auth_generation`,
`key_generation`, `transport_generation`, `recipient_seat`, `recipient_cert`,
`recipient_key` (32-byte string), `publisher_cert`, and
`purpose: "room-generation-secret"`. Numeric fields are unsigned integers.
The signature covers domain `cowchat/v3/room-key-envelope/v1` followed by
canonical CBOR `[scope_bstr, wrapped_bstr]`. Existing HPKE info is unchanged.

Use the existing signed HTTP request projection headers
`x-cowchat-certificate`, `x-cowchat-request`, and `x-cowchat-signature`.
PUT must be signed by `publisher_cert`. Only the current room owner's seat with
current `manage` membership may publish. A reader, builder, actor, or caller
claiming to be a key administrator cannot publish. Delegated key administration
needs its own authenticated provisioning path before this rule can expand.

`GET /rooms/{room}/key-envelopes/{recipient_identity_id}/{key_generation}?transport_generation={n}`
requires an empty body and a request signed by that exact recipient certificate.
It returns the same JSON envelope with `Cache-Control: no-store`. No listing or
anonymous read endpoint exists. Numbers and query spelling must be canonical;
PUT accepts no query. Duplicate authentication headers fail closed. Body size is
bounded to 16 KiB. Invalid authorization or unavailable envelopes return 401;
request nonce replay or conflicting publication returns 409.

## Authorization and retry behavior

Each transaction reads current enrolled credentials and room generations. It
checks both recipient `read` and publisher `manage` rights and expiry, the
publisher's owner wallet/seat, chain, recipient role, and readable generation
floor. The recipient encryption public key comes from its enrolled identity
certificate; its digest must equal the enrolled identity ID. Caller-supplied
scope must exactly match scope built from that state. The detached signature is
verified before storing or returning the envelope. No network or consensus I/O
runs in these transactions.

The local immutable slot is `(room, recipient_cert, key_generation,
auth_generation, transport_generation)`. A retry uses a fresh HTTP nonce and
exactly the same scope, wrapped bytes and detached signature. A fresh HPKE
encapsulation into the same slot conflicts, even if it encrypts the same secret.
Failed publication rolls back the request nonce. Store restart retains both
nonce fences and sealed slots. A transport/auth generation change requires a
new signed scope and uses a new slot; it never reinterprets an old scope.

Retrieval rechecks current membership and publisher authority. Removing either
credential, raising the recipient's history floor, or expiring either applicable
credential denies retrieval. Existing recipients can retain already decrypted
keys and plaintext. Rotate room keys to exclude removed recipients from future
traffic. The service cannot prove that an authorized owner encrypted the correct
room secret, or that the HPKE ciphertext actually targets its claimed key; it
checks signed scope and ciphertext shape, and the recipient verifies/decrypts.

A consumer must compare the returned scope with independently authenticated
current room/recipient/publisher state, verify the detached signature using that
publisher's independently resolved public key, and only then call
`keys::unwrap_room_key`. Neither the public key nor expected scope may be trusted
merely because they arrived alongside the envelope.

## Current limits and proof

Owner bootstrap and [wallet-issued builder enrollment](builder-enrollment.md)
are available through HTTP. A separate end-to-end test now uses both real
certificate factories before owner-to-builder envelope delivery and encrypted
builder append. The original builder authorization test still explicitly seeds
authenticated fixture state and is not an identity-issuance proof. Automatic
credential renewal and recipient private-key recovery remain client/runtime
prerequisites. The restart test proves persistence
of sealed service records, not private-key recovery or browser-closed renewal.

Publisher renewal within an otherwise unchanged slot is not an update operation:
the existing slot stays immutable. If its publisher expires, retrieval fails
closed until an authorized new recipient certificate or room authorization/key
generation yields a new slot. The broader credential-rotation workflow must
handle this before unattended production use; this service alone does not
provide automatic renewal.

Five tests exercise actual HTTP owner enrollment/publication/retrieval and local
HPKE verification/decryption, conflicting encapsulation, exact retry and replay,
wrong chain/room/seat/certificate/key/auth/transport scope even when re-signed,
recipient-key substitution, detached signature/ciphertext tampering, removed
members, expiry, recipient-only reads, builder publication denial, live history
floor checks, durable restart, transport changes, and injected transaction
failure with nonce rollback. The HTTP test checks only sealed bytes are stored;
this is not a general production logging or key-custody audit.

Run `cargo test -p cowchat-server --lib key_envelope` with the repository's Rust
1.93 toolchain. There are no CBSS, gateway-provider, or consensus calls in this
service. Production actor/door release remains separately gated by the reviewed
[key-delivery design](room-key-delivery-design.md).
