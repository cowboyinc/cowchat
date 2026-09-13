# Local M-ECHO composition

This runs three separate processes: the real Cowchat service and durable webhook
worker, the real generic gateway router/dispatch/proxy, and the real harness
`cowchat_wake` handler. It uses explicit test-only credentials, in-memory gateway
discovery, and a fixture runtime grant. It starts no node and makes no consensus
calls. Production actor enrollment and secret delivery are separate boundaries.

The local implementation cohort is:

| Repository | Branch | Reviewed implementation |
| --- | --- | --- |
| Cowchat | `jw/dashboard-cowchat-room-wake` | `c4a487b` |
| Gateway | `jw/dashboard-room-wake-forwarding` | `75986c8` |
| Harness | `jw/harness-cowchat-wake` | `4856e05` |

Build each example in its own repository with Rust 1.93. The harness dependency
pin is the real Cowchat git commit; no local path patches are needed. Unpublished
local commits must be available to Cargo's git cache before using `--offline`.

```sh
# Cowchat checkout:
cargo build --offline --locked -p cowchat-server --example cowchat_room_fixture
# Gateway checkout:
cargo build --offline --locked -p gateway-server --example cowchat_gateway_fixture
# Harness checkout:
cargo build --offline --locked -p harness-serve --example cowchat_harness_fixture
```

From the Cowchat checkout, with these sibling worktree names (adjust paths if your
checkouts are elsewhere):

```sh
python3 scripts/local_m_echo.py \
  --room-bin target/debug/examples/cowchat_room_fixture \
  --gateway-bin ../gateway-room-wake-forwarding/target/debug/examples/cowchat_gateway_fixture \
  --harness-bin ../harness-cowchat-wake/target/debug/examples/cowchat_harness_fixture
```

The driver creates an isolated temporary directory and loopback ports. It retains
a report, encrypted room SQLite database, ciphertext trigger, pointer-only
execution log, and process stderr. It stops all fixture processes on exit,
including on failure. It does not affect the local collaboration server.

The binary assertions cover:

1. A signed encrypted mention persists; an exact ciphertext retry returns its
   original receipt, and an independently encrypted candidate with the same ID
   conflicts.
2. The service is killed with one durable wake pending. The same dispatch ID and
   exact stored wake bytes survive service restart and receiver failure.
3. After appending its reply, the harness process crashes before acknowledging the
   gateway request. Restart causes a second execution with fresh encryption; the
   sink retains exactly one visible reply and the wake is eventually acknowledged.
4. The returned reply verifies under the bound actor's key and decrypts locally.
   The driver sees exactly the request and reply, and checks captured state/logs
   for the fixture plaintext. A forged wake through the gateway is refused before
   any room access or runtime execution.
5. Every wake travels through generic gateway dispatch into the actual harness
   handler. No Telegram-specific code or external provider API is involved.

The observed stderr streams were empty; the pointer-only execution log contained
both attempts. This does not establish privacy for production logging configurations.

The runtime's keys and authorization are explicit fixture inputs, not a live PKE
release proof. The gateway route/runner discovery is an in-memory fixture; it does
not prove deployed registry freshness. The handler is a fixture callback rather
than a generic Python executor. There is no paid-effect or exactly-once charge
claim, and no production default serving is enabled. This is the first local flow
proof, not completion of the full non-UI implementation or launch scope.
