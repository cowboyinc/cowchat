# Signed room client and reply recovery

`cowchat_client::seated` provides HTTP operations for an already provisioned seat.
Its client retains public room/seat metadata; callers lend the signing seed and
room-generation secret to individual operations. Runtime key release and compute
authorization remain separate integration work. Nothing in these operations
reads or writes consensus state.

`verify_wake` checks the exact Standard Webhooks body with the binding's secret,
rejects duplicate signature headers, enforces a five-minute timestamp window, and
validates room, transport generation, dispatch identity and cursor bounds. Verify
before fetching history or executing a handler. A valid wake authenticates a
notification; it does not authorize paid execution. The secret is the raw UTF-8
bytes configured for the subscription, matching the service's signer.

`prepare_reply` derives one stable reply ID from chain, room, seat and triggering
message. New executions may choose different text and encrypt with fresh random
nonces. Save `PreparedReply::bytes()` before sending; `restore_reply` validates and
recovers those exact ciphertext bytes after restart. Each HTTP retry signs a fresh
request nonce around the unchanged record.

`submit_reply` accepts an exact append retry. If another candidate already won the
same ID, it fetches that ID and verifies the existing record's signature and reply
scope against the provisioned seat before acknowledging it. Failure to recover an
authenticated winner remains an unresolved conflict. This permits repeated
handler execution while exposing one room reply; it provides no deduplication
guarantee for paid or non-idempotent external effects.

`read_ciphertext_page` returns ciphertext. The runtime must authenticate each
sender using independently provisioned membership credentials before decrypting
or using the content. The room service is not a source of trusted sender keys.

```sh
cargo test --offline --locked -p cowchat-server --lib client_tests
```

These tests exercise the actual HTTP room sink with explicit credential fixtures:
changed/stale/foreign/duplicate wake headers are rejected; independently sealed
reply candidates share an ID but have different nonces; recreating the client and
retrying persisted ciphertext preserves the first reply; a conflicting candidate
recovers that signed winner; a forged winner is rejected. The lost acknowledgement
is simulated by discarding a successful response, not by injecting a network
failure. This is not yet a gateway-triggered actor run or proof of runtime secret
delivery. The complete worker/gateway/runtime composition remains to connect.
