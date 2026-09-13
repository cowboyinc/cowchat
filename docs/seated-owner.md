# Seated owner enrollment, signed append, and history

This local slice proves real wallet-authenticated owner enrollment followed by an
encrypted, signed record through the HTTP service. It does not run a node or write
consensus state.

```sh
cargo test --offline --locked -p cowchat-server --lib owner_enrolls_through_real_http_then_posts_signed_encrypted_record
cargo test --offline --locked -p cowchat-server --lib seated_append_auth_nonce_idempotency_and_revocation_share_one_transaction
cargo test --offline --locked -p cowchat-server --lib seated_signed_history_returns_verifiable_ciphertext_and_binds_cursor_query
```

`POST /rooms/{room_id}/owner` requires the existing room owner's authenticated
`x-cowchat-key` and JSON containing `identity`, `identity_signature`, `membership`,
and `membership_signature`. These are unpadded standard-base64 certificate/signature
bytes. Both certificates must be wallet-signed, generation zero, for the same owner;
the membership must name this room and grant manage/read/write. The service checks
wallet signatures, roles, audience, network, scope, and expiry. Only an empty private
room can switch to seated mode. The same enrollment retries; different credentials
or a replay after a generation change cannot overwrite the installation. Public
certificate proofs are retained for subsequent certificate reads, without wallet
private keys or room decryption keys.

`POST /rooms/{room_id}/messages` accepts the spec's flat JSON v3 sealed record.
`x-cowchat-request` is the base64 canonical-CBOR signed request projection;
`x-cowchat-signature` is its base64 Ed25519 signature. The signature binds the actual
method, raw path/query, and body bytes. The record's signature also binds the header
and ciphertext. Membership/generation checks, nonce consumption, message idempotency,
and wake obligations commit in one SQLite transaction. The reply contains only
message ID, accepted status, and sequence. A transport retry uses a fresh request
nonce around the identical sealed record; it returns the original append result.

`GET /rooms/{room_id}/messages?transport_generation=0&after=0&limit=100` uses the
same request-signature headers plus `x-cowchat-certificate` identifying the current
credential. The transport generation is required; `after` defaults to zero and
`limit` defaults to 100 (maximum 100). The raw query is signed, the request body
must be empty, and a fresh request nonce is required for each read. The service
checks current membership, read rights, expiry, and transport generation before
consuming the nonce and returning a page. Pages contain ciphertext records and
`cursor: {room, transport_generation, position}`, with a 4 MiB record budget.
The test verifies and decrypts the returned record on the client, and exercises
altered queries, unexpected bodies, replay, wrong generations, and revocation.
The server never receives a room decryption key.

Legacy bearer writes, history, and room access cannot bypass seated authentication.
Legacy rename/destroy and re-enable paths are also blocked. Seated messages,
receipts, and blob rows are exempt from legacy age retention.

The owner endpoint, signed append, and owner history are implemented; signed
subscription management, owner credential renewal/revocation endpoints, actor enrollment from a
verified finalized control record, and the full actor wake/reply proof remain to
be connected. Tests that directly install trusted actor-like contexts are explicitly
provisioning fixtures, not evidence of live actor identity verification.
