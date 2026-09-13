# Signed room diagnostics

`GET /rooms/{room}/diagnostics?transport_generation=N` returns a local metadata
snapshot for a currently authorized reader. It uses the same certificate,
request projection, signature and replay protection as signed history reads.
Sign the exact path and query with an empty body; send `x-cowchat-certificate`,
`x-cowchat-request` and `x-cowchat-signature`. Unknown query fields or a body are
rejected. Each attempt needs a fresh request nonce. Successful responses use
`Cache-Control: no-store`.

The version 1 response contains only:

- `transport`: `kind: "sqlite"`, current `generation` and `local_tip`.
- `authorization_generation` and `key_generation`.
- `subscription`: null when this seat has no seated subscription, otherwise
  `state` (`active`, `failed`, `disabled`, or `unknown`), `binding_current`, and
  `pending_wakes`. Binding is current only when the subscription's certificate,
  authorization generation and transport generation match the authenticated
  reader and room. The count includes pending seated wake rows for this seat,
  including retries or rows awaiting expiry processing. It is not delivery lag,
  an execution count, or proof that a consumer can currently receive them.
- `checks`: `archive: "unsupported"` and `production_runtime: "unsupported"`.

The snapshot does not certify health or independent durability. A local tip is
not an archive watermark or final SENT acknowledgement. It performs no repair,
funding lookup, remote probe, key release or consensus operation. Its only write
is local request replay bookkeeping in the snapshot's SQLite transaction.

No other seat's subscription or backlog is returned, including to an owner.
The query never loads message bodies, ciphertext, webhook URLs, secrets, wake
payloads or error strings. Stored subscription states are mapped to the fixed
allowlist; unrecognized values become `unknown`. Error responses contain status
codes only: 400 malformed query/body, 401 invalid authority/signature/binding,
409 replay, 500 internal failure (framework request size limits may return 413).

The tests exercise real signed HTTP reads against the service's SQLite store,
own-seat isolation, empty and pending subscriptions, status/binding changes,
replay, revoked/expired credentials, incorrect room/generation/signature, and
secret canaries in persisted data. Credentials and keys are public fixtures;
these tests do not establish production provisioning or runtime health. CLI
presentation and credential loading are a separate follow-up.

## Rust client

`SeatedHttpClient::diagnostics(signing_seed)` returns a typed `RoomDiagnostics`
snapshot. It reuses the client's signed requests, fresh nonces, restricted origin,
disabled redirects and timeout. It borrows the enrolled seat's signing seed for
the operation and never needs a room decryption key. Provisioning that seat is
the caller's responsibility; the method does not create or repair credentials.

The client accepts at most 16 KiB of response bytes. It rejects unknown versions,
unknown or duplicate fields, missing required fields, negative counters, unknown
enum values and a transport generation different from its configured seat. The
nullable subscription field must still be present. HTTP refusals return only a
status code; parser/transport errors never include a server body or raw URL.
Unsupported checks remain typed `Unsupported` values, not a successful health
assessment. The snapshot is a report from the configured service, not a signed
archive receipt or independently verified state. Existing history reads retain
their separate 5 MiB response limit.
