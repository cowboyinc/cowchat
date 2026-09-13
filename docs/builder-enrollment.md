# Wallet-issued builder enrollment

`POST /rooms/{room}/builder` installs the room owner's separate `#builder` seat
using wallet-issued identity and membership certificates. It is an initial
enrollment path, not an automatic credential-renewal authority. It performs no
network or consensus calls and grants no permission to start paid turns.

The body contains the same four unpadded standard-base64 fields as owner/actor
enrollment: `identity`, `identity_signature`, `membership`, and
`membership_signature`. The request's `x-cowchat-request` projection and
`x-cowchat-signature` must be signed with the **builder identity's Ed25519 key**.
The room-owner API key or owner's Ed25519 signature cannot replace that proof of
possession. Duplicate authentication headers, a query string, and bodies over
512 KiB are rejected.

The room must already have an authenticated owner with current `manage` rights.
Inside one local SQLite Immediate transaction, the service obtains the chain,
room owner wallet and current generations from enrolled room state, then verifies:

- Both certificates are signed by that wallet using their existing separate
  identity/membership domains. Delegated-admin membership is not accepted here.
- The identity has `role: builder`, the canonical owner wallet `address`, an
  Ed25519 signing key, its own X25519 encryption key, the room chain, and a finite
  unexpired identity credential. Its identity generation equals the current room
  authorization generation for this bootstrap path.
- Membership names this exact room and `{owner_address}#builder`, has exactly
  `read` and `write` rights, the current authorization generation, no door or
  forwarded-sender fields, and `from_gen <= current room key generation`.
- The signed HTTP request covers the actual body and target, proves possession
  of the enrolled signing key, and has a fresh local nonce.

The resulting credential's expiry is the earlier applicable identity/membership
expiry. The stored context attributes posts to `role: builder` and the separate
builder seat. History and key-envelope access enforce the stored generation
floor through the existing signed endpoints. Credential installation, nonce
claim, and a content-free enrollment receipt commit atomically.

There is one bootstrap receipt per room authorization generation. A fresh-nonce
retry of the exact certificate/signature tuple returns the existing seat and
certificate only while that credential remains installed. A different tuple
conflicts. Removing the credential leaves the receipt, so an old signed request
cannot recreate the removed seat, including after service restart. An existing
builder inserted by another authenticated path is not overwritten. No room
subscription, conversation link, invocation grant, or room key is created by
this endpoint.

This intentionally does not rotate an existing builder's credentials. A new
certificate in the same authorization generation conflicts even when it has a
valid wallet signature. Owner-approved generation transitions and a separately
reviewed renewal mechanism must handle replacement. Chad's until-revoked access
policy remains the product requirement; this bootstrap alone does not implement
automatic renewal or browser-closed private-key recovery. It never retains a
wallet private key or treats a builder signing key as a delegated identity issuer.

## Verification

`cargo test -p cowchat-server --lib builder_` includes a real HTTP flow with
public test wallet keys: owner HTTP enrollment, builder HTTP enrollment, owner
publication of an HPKE envelope to the builder's certified encryption key,
builder-only HTTP retrieval, independent scope/signature verification and local
HPKE unwrap, then a builder-signed encrypted append using the delivered secret.
No SQL credential seeding occurs in this flow. This proves the enrolled service
path, not production key custody, a live wallet UI, or automatic renewal.

Additional tests re-sign wrong chain, address, room, seat, role, generation,
rights, history floor and expired certificates; reject another wallet's
signatures and the wrong request-signing key; cover exact retry and replay;
exercise injected receipt-write failure with credential/nonce rollback; and
reopen durable state before confirming removed credentials cannot be restored.

The [key-envelope contract](room-key-envelopes.md) and
[key-delivery design](room-key-delivery-design.md) describe delivery and the
separately gated CBSS session-release work.
