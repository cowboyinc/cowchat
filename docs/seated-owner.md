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

An optional signed `message_id=<canonical UUID>` query restricts that page to one
record, within the same room, cursor, membership and key-generation bounds. This
lets a reply producer recover a conflicting append's authenticated winner without
scanning the room. An absent or inaccessible record returns an empty page.

Legacy bearer writes, history, and room access cannot bypass seated authentication.
Legacy rename/destroy and re-enable paths are also blocked. Seated messages,
receipts, and blob rows are exempt from legacy age retention.

`POST /rooms/{room_id}/subscriptions` uses the same signature and certificate
headers. Its JSON contains a client UUID `subscription_id`, `transport_generation`,
`webhook_url`, a per-subscription `secret` (32–512 bytes), and optional `after`.
Omitting `after` starts at the current tip. This slice supports one subscription
per authenticated seat, for explicit mentions in `message` records with a nonzero
wake hint. It cannot subscribe as another seat or enable broader data/broadcast
filters. Creating a notification subscription does not grant compute authority.

Membership is checked before URL validation and again in the local transaction
that consumes the nonce and installs the subscription, backlog, and immutable
CloudEvents wake payloads. Retries with a fresh signed request nonce and the exact
same body acknowledge the existing subscription without resetting or reviving it.
Changed inputs conflict. Wakes contain only room/message/cursor/dispatch pointers,
use Standard Webhooks signatures, and preserve their body and ID across retries.
The worker rechecks local membership, expiry, and generations on each attempt.
An already-sent pointer may race revocation; subsequent history access still
requires current credentials. Legacy subscription APIs cannot manage seated rows.

```sh
cargo test --offline --locked -p cowchat-server --lib seated_subscription_http
cargo test --offline --locked -p cowchat-server --lib seated_subscription_default_filters
```

The HTTP test uses real owner enrollment and a local HTTP receiver, verifies
signed pointer-only redelivery, checks lost-response create retries, injects a
backlog-write failure to verify full rollback including the nonce, and checks
revocation before delivery. It does not execute an actor or establish its identity.
Signed subscription update/delete/repair are implemented through
`POST /rooms/{room_id}/subscriptions/{subscription_id}/lifecycle` with the same
certificate and request-signature headers. The signed JSON is:

```json
{
  "operation_id": "40000000-0000-4000-8000-000000000001",
  "transport_generation": 0,
  "expected_revision": 0,
  "action": {"kind": "repair"}
}
```

Creation returns revision zero. Each mutation compares and increments the local
revision. `action.kind` is `repair`, `delete`, or `update`; update requires both
`webhook_url` and `secret` inside `action`. It preserves status, cursor and queued
wakes. Repair explicitly activates the binding, backfills messages after its
acknowledged cursor, and resets eligible unacknowledged attempts. Retained wakes
keep their delivery IDs and exact CloudEvent bytes. Missing history is not
invented; existing retention/deadline policies still apply.

Only a current credential for the same seat can mutate a binding. Repair can
rebind an independently enrolled replacement certificate and excludes records
below its readable key floor. Update requires the existing current certificate;
renewal must repair first. Transport generation changes require deleting the old
binding and creating a new one, never rewriting the generation in retained wakes.

Repeat the exact operation body with a fresh signed request nonce to recover its
historical receipt without applying the action again. Conflicting operation UUIDs
or revisions return 409. Receipt recovery still requires current membership and,
for update, endpoint validation. Delete atomically removes the subscription and
its pending wakes, freeing the seat. A durable tombstone prevents reuse of the
old subscription UUID. Receipts/tombstones contain IDs, seat, revision/status,
cursor and a request digest, never endpoint/secret values or room content.

Standard Webhooks timestamp/HMAC headers are generated per attempt. An attempt
started after endpoint/secret update uses the new configuration; an already-sent
attempt retains its old headers. The durable pointer body and ID never change.
The worker checks the local revision before sending and atomically compares it
again with outcome writes. A late acknowledgement/failure from an old revision
cannot advance the new cursor or disable a repaired subscription. Delete cannot
recall a network request already in flight. All lifecycle state is local SQLite;
there are no consensus reads or writes in these operations.

`cargo test --offline --locked -p cowchat-server --lib seated_subscription`
covers real HTTP lifecycle requests, nonce replay and conflicting retries,
delete/recreation, receipt persistence across restart, transaction rollback,
revocation during URL validation, replacement credential history bounds, and
an actual blocked webhook response released after repair/secret rotation.
Broader owner-approved filters remain to implement.

The owner endpoint, signed append, owner history, and mention-subscription creation
are implemented. [Actor enrollment](actor-enrollment.md) now verifies a fetched
finalized control proof before installation. Owner credential renewal/revocation
endpoints remain to be connected. The [local actor wake/reply proof](local-m-echo.md)
uses fixture keys and does not establish production key release.
Tests that directly install trusted actor-like contexts are explicitly
provisioning fixtures, not evidence of live actor identity verification.
