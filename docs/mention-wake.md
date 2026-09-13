# Local mention-wake delivery proof

This slice runs an authenticated Cowchat client, the real SQLite-backed room service,
and a local HTTP receiver. It does not start or write to a consensus service.
It proves mention-triggered webhook delivery; it does not yet implement v3 seat
authorization, actor execution, or the encrypted actor reply.

Run from the repository root:

```sh
cargo test --offline --locked -p cowchat-server --test integration_tests test_mention_wake_authenticated_client_backfill_and_live_delivery -- --nocapture
cargo test --offline --locked -p cowchat-server --lib mention_wake_lost_http_ack_and_restart_reuse_authenticated_dispatch
```

The first test sends messages through the authenticated client/server API and checks
both historical and live explicit mentions. Text containing `@actor` and a caller's
`metadata.mentions` do not trigger a wake. The HTTP receiver verifies the signature.

The second test consumes a real HTTP request and drops the connection before
acknowledging it, drops the service state, reopens SQLite, and starts the delivery
worker. The retried POST carries the same body, `dispatch_id`, and `webhook-id`.
Its timestamp/signature may refresh. Receivers must verify the signature and use
the dispatch ID to deduplicate effects: delivery can repeat.

The CLI exposes `cowchat sub create ROOM --only-mention ID` alongside the existing
URL, secret, kind, and sender filters. The ID is an exact explicit mention target;
it is not yet proof of a Cowboy seat. Existing subscriptions without this filter
retain their behavior.

Initial backlog and subscription creation commit together. An acknowledgement
consumes the pending delivery and advances the cursor in one SQLite transaction.
A stale backfill cannot recreate an already-acknowledged sequence.

The existing legacy service retention/retry limits still apply. Long-lived seated
room retention and key authorization are subsequent work, not claims of this proof.
