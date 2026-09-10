# cowchat-crypto

Cowchat v3 encoding, signatures, encryption, and key wrapping, shared by Rust
consumers and future browser/Swift bindings. There is no network, database, or
room-service dependency. Public operations accept byte slices, strings, and
numeric values and return owned bytes or stable numeric `Error` discriminants.
These are Rust functions; an actual C/wasm/Swift ABI is a later binding step.

The crate does **not** turn a valid signature into room membership or permission
to spend. Callers supply authenticated state and enforce the service policies.

| Module | Operations | Caller responsibility |
| --- | --- | --- |
| `canonical` | Validate or order the bounded deterministic CBOR profile | Use schema validation too; generic CBOR validity is not an envelope/certificate. |
| `envelope` | Header validation, signing bytes, verify, seal, open | Resolve signer from an authenticated certificate; enforce seat/role/door/system-record rights and current generations. |
| `certificates` | Three certificate layouts, IDs, signing bytes, signature and supplied-context verification | Resolve current owner/admin authority, actor controller commitments, membership, revocation, and invocation budget. |
| `request` | Signing, signed projection validation, raw received request binding | Use `verify_received` for HTTP, then atomically claim its nonce token in durable storage before effects. |
| `keys` | Legacy-compatible HKDF, HPKE wrapping/unwrapping | Store and rotate the generation secret; erase owned private inputs and returned plaintext/key buffers. |

## Wire and trust inputs

Header and certificate input bytes are the already deterministic CBOR specified
by `fixtures/v3`. `canonicalize` sorts text map keys but rejects non-preferred
integer/length encodings, duplicates, tags, floats, and unsupported types.
`validate` additionally rejects incorrectly ordered maps. Public signatures
never silently canonicalize received bytes.

Certificate kinds are scalar constants: `IDENTITY=1`, `MEMBERSHIP=2`,
`INVOCATION=3`. Certificate verification accepts this trusted-context CBOR map:

```
{chain_id: uint, room: text, gen: uint, now_ms: uint,
 wallet_address: bstr(20), admin_key: null | bstr(32)}
```

The caller must independently resolve `admin_key` from a current owner identity
delegation and manage membership. It must not copy the key from the certificate
under test. `gen` means the current authorization/membership/invocation
generation appropriate to that certificate kind. Verification requires exact
generation equality; accepting a newer or older generation would bypass the
current revocation state. For actor/door identities,
signature verification must be followed by the finalized actor-controller and
certificate-commitment check. No production API accepts an `admin_has_manage`
or `actor_is_authorized` boolean from a caller or wire record.

`verify_projection` returns a canonical CBOR nonce token
`[key_bstr32, nonce_bstr16, retain_through_ms]`. Only a server-side atomic
insert-if-absent keyed by (key, nonce) confers replay protection. Keep the entry
through the returned millisecond inclusive. Use `verify_received` to additionally
bind the projection to the exact HTTP method, raw origin-form target, and body.
HTTP adapters must reject non-UTF-8 request targets before this call, never
convert them with a lossy decoder or normalize/re-encode their bytes.
Room-service authorization is required even after both checks pass.

`seal` ignores the supplied header nonce and replaces it with fresh OS
randomness. Its result is CBOR `[header_cbor_bstr, cow1_body_text, signature_bstr]`.
Persist those exact bytes before append; retries reuse them. Calling `seal`
again produces a different record and is not an idempotent retry. HPKE wrapping
also generates fresh ephemeral randomness internally and returns
`[enc_bstr, ciphertext_bstr]`. There is no deterministic production nonce API.

The 32-byte HPKE plaintext and CBSS value are the generation secret, not the
already-derived AEAD key. Both feed the one `derive_room_key` implementation.
`cowchat-core::crypto` re-exports it so existing `cow1` clients keep their bytes.

## Bounds and evidence

Signed CBOR is capped at 64 KiB, depth 16, and 4096 total values. Length and
nesting are preflighted before general-purpose decoding allocates. Envelope
body text is capped at 1 MiB. These are crate admission limits, not promises
about transport capacity: the server's existing 64/256-KiB caps and CBQS's
256-KiB envelope limit can reject a record accepted by this crate.

Tests consume the 77 fixed Python-generated vectors and exercise actual
production APIs. Additional tests cover raw HTTP binding, fresh seal/open,
fresh HPKE, hostile lengths/nesting, and trusted admin-key separation. This is
not yet full service conformance, durable replay/outbox proof, a browser/Swift
binding build, or a live finalized-state/CBSS release test.

```sh
cargo test -p cowchat-crypto
cargo test --workspace
cargo clippy -p cowchat-crypto -p cowchat-core --all-targets -- -D warnings
cargo fmt --all -- --check
```
