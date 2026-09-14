# Local M-ECHO composition

This proof runs the actual Cowchat HTTP/SQLite service and webhook worker, the
generic Gateway router and runner proxy, Harness room execution, supervised CBSS
fixture writers and HTTP release, and the Cattle Guard PostgreSQL coordinator,
journal, and serving accounting. It uses explicit fixture keys, funding, model
output, and discovery. It starts no node and makes no consensus calls.

Use Rust 1.93 and a disposable PostgreSQL database named `room_runs_test` on an
explicit `127.0.0.1` port other than 5432. The Harness build must resolve to the
Cattle Guard and CBSS revisions containing the room coordinator, builder-seat
migration, and supervised process fixtures.

Build the fixture binaries in their respective checkouts:

```sh
# Cowchat
cargo build --offline --locked -p cowchat-server --example cowchat_room_fixture

# Gateway
cargo build --offline --locked -p gateway-server --example cowchat_gateway_fixture

# Harness
cargo build --offline --locked -p harness-room-journal \
  --example cowchat_paid_harness_fixture
```

Run the driver from the Cowchat checkout, adjusting binary paths for the local
worktrees:

```sh
python3 scripts/local_m_echo.py \
  --room-bin /path/to/cowchat_room_fixture \
  --gateway-bin /path/to/cowchat_gateway_fixture \
  --harness-bin /path/to/cowchat_paid_harness_fixture \
  --database-url postgres://postgres@127.0.0.1:55440/room_runs_test
```

The driver creates isolated state and loopback ports and performs this sequence:

1. Harness publishes one verified actor seat and one verified builder seat from
   a shared CBSS fixture publication.
2. Cowchat installs mention-only subscriptions for both seats and appends two
   encrypted triggers. Exact ciphertext retries return the original receipts;
   changed ciphertext under the same message ID conflicts.
3. Cowchat stops with both seated wakes durable in SQLite. Gateway and Harness
   start, a forged Standard Webhooks request is rejected before durable
   admission, and Cowchat restarts and drains the original wake obligations.
4. Cattle Guard admits only authenticated room references. The worker decrypts
   in Harness, uses the paid model path, stages signed usage and an encrypted
   reply, closes accounting, and then the Harness process exits before Cattle
   Guard can mark that room run complete.
5. Harness restarts from PostgreSQL. It reconciles the first seat without a
   second model call, executes the other seat once, and finishes both stable run
   references. Cowchat verifies both signatures and decrypts exactly two replies.

The final report asserts one model call per seat, exactly one recovered lease
(the two lease epochs are 1 and 2), zero held accounting balance, both signed
charges, no pending wakes, live CBSS writers, unchanged dispatch IDs, and zero
consensus calls. It also scans captured files and the relevant PostgreSQL
content sinks for the fixture plaintext canaries.

The report does not claim real CBQS transport, CBFS archive, provider delivery,
separately operated CBSS, production key custody or funding, or deployed-network
readiness. No provider-specific gateway API participates in the proof.
