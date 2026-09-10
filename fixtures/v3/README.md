# Cowchat v3 M0 crypto fixtures

These are public test vectors and an executable **reference contract**, not
production room authentication or evidence that the hosted service supports v3.
The existing server and client paths are unchanged. WP1/WP1b must run these
vectors through their actual implementations. The manifest lists the remaining
contracts and integration work; passing this suite does not finish M0.

## Run

From the cowchat checkout:

```sh
cargo test -p cowchat-core
cargo clippy -p cowchat-core --all-targets -- -D warnings
cargo fmt --all -- --check
```

The checked-in JSON is consumed by Rust without a Python installation. To
regenerate it, create an isolated Python environment, install
`fixtures/v3/requirements.txt`, then run `fixtures/v3/generate.py`. Regeneration
must leave the JSON bytes unchanged. All keys and deterministic randomness in
this folder are public test inputs and must never be used in a real room.

## Frozen encoding rules in this slice

- Signed maps use RFC 8949 section 4.2.1 core deterministic ordering: bytewise
  lexicographic order of encoded keys. The spec's displayed field order is for
  readers. This profile has text map keys and no floats, negative integers,
  tags, indefinite lengths, duplicate keys, or trailing bytes. Optional fields
  are present as null; unknown signed fields are rejected.
- Domains are raw UTF-8 prefixes. Requests sign the domain followed by the
  deterministic CBOR array `[method, raw_target, body_sha256, timestamp_ms,
  nonce]`. The hash and nonce are byte strings (32 and 16 bytes). Method is
  uppercase; target is the unmodified ASCII origin-form path and query. No
  percent decoding, query sorting, or duplicate-query-key normalization occurs.
- Records sign the envelope domain, deterministic CBOR header, and SHA-256 of
  the UTF-8 **full `cow1:` body string**. Header nonce is canonical unpadded
  standard base64 of 12 bytes and must match the nonce in the body. Encryption
  uses the existing `cowchat-e2e-v1:` HKDF label and adds canonical header AAD.
- Wallet signatures use Keccak-256 of domain plus CBOR, recoverable secp256k1
  `r || s || recid` (65 bytes, raw recid 0..3), low-S only. Address derivation
  matches CBFS. This inherits CBFS's signature policy, **not** its distinct
  big-endian certificate serialization. Requests/records and delegated admin
  membership signatures use Ed25519 over signing bytes directly.
- Certificate IDs are SHA-256 of domain plus CBOR, excluding the signature.
  Certificate JSON includes `byte_fields` listing hex fields projected to CBOR
  byte strings. A valid invocation budget is a decimal string in JSON and a
  fixed 16-byte big-endian u128 (wei) in CBOR. Negative budget cases deliberately
  use other shapes. Until-revoked membership is distinct from expiring owner
  or builder identity credentials.
- HPKE is RFC 9180 base mode: X25519/HKDF-SHA256/ChaCha20Poly1305, info = roomkey
  domain plus deterministic CBOR `[room, gen]`, empty AAD. Wrapped plaintext
  and CBSS content are the same 32-byte generation secret, which is then passed
  to `derive_room_key`; neither path wraps an already-derived AEAD key.
- Requests accept timestamps at either edge of an inclusive 300-second window.
  A nonce is retained per certificate key through signed timestamp + 300 seconds
  inclusive. A request first accepted at the future edge needs up to 600 seconds
  of retention. Expired entries can be evicted only after that bound; replay
  thereafter fails timestamp validation.

## Evidence limits

Python cbor2, cryptography, eth-keys, and pyhpke generate fixed bytes; Rust
ciborium, RustCrypto, k256, and hpke consume them. This provides independent
implementation checks for the covered cryptographic/encoding operations. A
browser or Swift wrapper of the same Rust crate will establish binding parity,
not another independent crypto implementation.

The certificate tests supply an owner/admin trust context. They do not prove
the delegation/manage chain, door forwarding authority, actor control record,
or finalized checkpoint lookup. The replay map is in-memory test state, not a
durable server cache. The CBSS input in the HPKE test is a supplied fixture,
not a live secret release. Member lifecycle, atomic key publication, history
cutoffs, outbox/archive recovery, raw HTTP-target extraction, and wire caller
integration remain separate gates.

Rulings were agreed in local `dashboard-impl` (UUID in manifest), messages
18, 21, 23, and 26. The canonical product specification remains
`dashboard-cowchat-spec.md` in the parent workspace.
