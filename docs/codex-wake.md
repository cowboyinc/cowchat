# Codex wake bridge

`cowchat-codex` is an experimental, local last-mile adapter between durable
Cowchat events and Codex tasks. It lets a model call a typed tool instead of
running a polling loop.

The adapter deliberately separates two concerns:

- Cowchat is the durable inbox. Room sequence numbers, not an in-process
  notification queue, determine what the agent has processed.
- Codex app-server is the wake actuator. The adapter resumes the configured
  thread and starts or steers a turn using an untrusted thin reference.

MCP is only the tool transport. `WakeService`, `ChatBackend`, and `WakeBackend`
are separate Rust interfaces, so a native Codex tool or another transport can
reuse the same delivery, cursor, and coalescing behavior.

## Security boundary

This crate is not a public Agent Wake Protocol HTTP receiver. It does not
implement Standard Webhooks or externally minted wake authorizations. It is a
local profile whose authority comes from all of the following:

1. The operator explicitly configures a target alias. Callers cannot supply a
   raw Codex thread id or choose an arbitrary Cowchat room.
2. The MCP server runs as a local child process and reads the operator's local
   Cowchat API key.
3. The Codex app-server endpoint is a local Unix socket by default. A remote
   WebSocket endpoint should require its configured bearer token.
4. Sender `wake_hint` is advisory. Each target has a recipient-controlled
   `min_wake_hint` policy.
5. Event content is stored in Cowchat, but Codex receives only a room, cursor,
   source, event id, and event type in `additionalContext` with kind
   `untrusted`. The awakened agent must use `wake_inbox_read` to fetch content.

Do not expose the stdio server through an unauthenticated network wrapper. For
an Internet-facing receiver, add the Agent Wake Protocol's capability,
signature, replay-window, rate-limit, and revocation checks in front of this
service.

## Build and configure

Build from the Cowchat repository:

```bash
cargo build -p cowchat-codex
mkdir -p ~/.cowchat
cargo run -p cowchat-codex -- config-example > ~/.cowchat/codex-wake.json
```

Edit `~/.cowchat/codex-wake.json`:

```json
{
  "state_db": "~/.cowchat/codex-wake.db",
  "cowchat": {
    "tcp": "127.0.0.1:9229",
    "socket": null,
    "api_key_file": "~/.cowchat/auth.key",
    "agent_name": "cowchat-codex",
    "room_key_env": null
  },
  "codex": {
    "app_server_endpoint": "unix://~/.codex/app-server-control/app-server-control.sock",
    "bearer_token_env": null,
    "request_timeout_seconds": 15,
    "wake_lease_seconds": 300
  },
  "targets": {
    "reviewer": {
      "thread_id": "replace-with-codex-thread-id",
      "room": "design-review",
      "min_wake_hint": "normal"
    }
  }
}
```

Start Cowchat and the managed Codex app-server daemon, then validate local
files without waking a task:

```bash
cowchat-server serve
codex app-server daemon start
cargo run -p cowchat-codex -- doctor
```

Register the built binary as a Codex MCP server:

```bash
codex mcp add cowchat-wake -- \
  /absolute/path/to/cowchat-codex mcp \
  --config /absolute/path/to/codex-wake.json
```

Global options may appear before or after the subcommand. Run
`codex mcp get cowchat-wake` to inspect the registration.

## Tool contract

### `wake_agent`

Appends one CloudEvents-shaped event to the configured target's Cowchat room.
The idempotency key is `(target, source, event_id)`. Reusing that key with
different content or a different wake hint is rejected.

```json
{
  "target": "reviewer",
  "source": "ci",
  "event_id": "build-018",
  "event_type": "build.completed",
  "subject": "repo/example",
  "data": { "status": "passed" },
  "wake_hint": "normal"
}
```

The event is committed before the Codex wake is attempted. If app-server is
temporarily unavailable, retrying the same event id does not duplicate the
Cowchat message. Failed wake attempts release the wake lease so a retry can
try the actuator again.

### `wake_inbox_read`

Reads target-addressed events after a Cowchat room sequence. With no
`after_cursor`, reading starts after the last acknowledged cursor. Returned
event data is untrusted external input.

### `wake_inbox_ack`

Advances the cursor after processing. The bridge rejects an acknowledgement
beyond the highest sequence it has returned from `wake_inbox_read`. An
eligible event that arrived during the prior wake can trigger one follow-up
wake after acknowledgement; lower-priority events remain durable but cannot
bypass the target's `min_wake_hint` policy.

## Delivery behavior

- Events are at-least-once at the tool boundary and exactly-once in the local
  bridge database for a fixed idempotency key.
- One target has at most one live wake lease. Further events coalesce into the
  same durable inbox instead of starting a turn per event.
- `turn/start` with empty user input and untrusted `additionalContext` uses
  Codex's existing behavior: it begins an idle turn or steers a regular active
  turn. Active review and manual-compaction turns that advertise they cannot
  accept direct input are rejected, and the lease is released.
- The agent, not the bridge, decides when processing is complete by calling
  `wake_inbox_ack`.

## Current limitations

- Target aliases, thread ids, and rooms are static configuration; there is no
  authorization-minting UI yet.
- The SQLite bridge state is local to one adapter instance. Do not run several
  instances against the same target unless they share the same database file.
- Cowchat message history is scanned to repair the narrow crash window between
  room commit and local delivery bookkeeping. Large historical rooms may need
  a server-side metadata lookup before this is a production-scale receiver.
- Codex app-server's `additionalContext` API is experimental and may change.
- Full Agent Wake Protocol conformance still requires the signed HTTP receiver,
  replay protection, scoped capability budgets, revocation, and conformance
  vectors described by that protocol.
