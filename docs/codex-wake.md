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
2. The MCP server runs as a local child process and connects through Cowchat's
   same-machine UDS/loopback trust boundary. Raw Cowchat TCP is rejected unless
   its address is loopback; the bridge does not support remote Cowchat over raw
   TCP. No local API key is required by default. The Unix socket is user-scoped,
   while loopback TCP is reachable by other local processes; use
   `--require-local-auth` on a shared or untrusted host.
3. The Codex app-server endpoint is a local Unix socket by default. Cleartext
   `ws://` is accepted only for loopback hosts. A remote endpoint must use
   `wss://`; configure `bearer_token_env` when that server requires a token.
4. Sender `wake_hint` is advisory. Each target has a recipient-controlled
   `min_wake_hint` policy.
5. Event content is stored in Cowchat, but Codex receives only a room,
   generation-bound cursor reference, source, event id, and event type as the untrusted
   `cowchat_wake_reference`. A separate, fixed `cowchat_wake_protocol` entry is
   marked as trusted application context and tells the task to read, process,
   and acknowledge the durable inbox. External event data never becomes
   application instructions.
6. The managed relay is disabled per target unless `relay` is explicitly true.
   It observes only that target's configured canonical room, ignores the
   recipient's stable `agent_id`, thinking pulses, and bridge envelopes, and
   remains bounded by the existing wake lease. Anyone allowed to post in that
   room can cause a wake, so room membership is part of the trust boundary.
7. Wake targets must be permanent rooms. For an encrypted room,
   `cowchat.room_key_env` must name a non-empty secret; `doctor --live` verifies
   that secret against the latest retained ciphertext when history is nonempty
   and reports an explicit unverified state for an empty encrypted room.

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
    "agent_id": "cowchat-codex-01989f2d-95e3-7b20-9a35-9c1e48295d41",
    "room_key_env": null
  },
  "codex": {
    "app_server_endpoint": "unix://~/.codex/app-server-control/app-server-control.sock",
    "bearer_token_env": null,
    "request_timeout_seconds": 15,
    "wake_lease_seconds": 300
  },
  "relay": {
    "poll_interval_ms": 1000
  },
  "targets": {
    "reviewer": {
      "thread_id": "replace-with-codex-thread-id",
      "room": "replace-with-canonical-room-uuid",
      "agent_id": "replace-with-unique-task-agent-id",
      "relay": true,
      "min_wake_hint": "normal"
    }
  }
}
```

`cowchat.agent_id` is a required, stable base identity for this bridge
installation. Generate it once (the `config-example` command does this), save
it with the config, and do not copy the value between installations. The bridge
derives distinct `<base>-mcp`, `<base>-relay`, and `<base>-doctor` Cowchat
identities so the three processes cannot evict one another. The base must be at
least 16 characters, and no target recipient `agent_id` may equal one of those
derived role IDs. The legacy shared value `cowchat-codex-bridge` is rejected.

`api_key_file` is optional in practice for the default local transport: a
missing file becomes an empty key. Keep it configured when the local server was
started with `--require-local-auth`. Remote Cowchat raw TCP is intentionally
unsupported; use a same-machine socket or loopback address.

Start Cowchat and the managed Codex app-server daemon, then validate local
files without waking a task:

```bash
cowchat-server serve
codex app-server daemon start
cargo run -p cowchat-codex -- doctor
cargo run -p cowchat-codex -- doctor --live
```

The local doctor validates JSON and the state database. `doctor --live` also
joins every configured canonical room and reads every configured Codex thread
without steering it or starting a turn. Its JSON reports structured
`room_readiness`, `thread_readiness`, errors, and a per-target `ready` value;
the command exits unsuccessfully if any target is temporary, inaccessible,
missing its encryption key, in `systemError`, or otherwise cannot be started or
steered safely. `codex.request_timeout_seconds` is one deadline for the entire
app-server connection, WebSocket handshake, initialization, write, and response
lifecycle; it is not reset for each RPC in the sequence.

Register the built binary as a Codex MCP server:

```bash
codex mcp add cowchat-wake -- \
  /absolute/path/to/cowchat-codex mcp \
  --config /absolute/path/to/codex-wake.json
```

Global options may appear before or after the subcommand. Run
`codex mcp get cowchat-wake` to inspect the registration.

For a target with `relay: true`, run one managed relay process under the same
operator account:

```bash
/absolute/path/to/cowchat-codex \
  --config /absolute/path/to/codex-wake.json relay
```

On first start, the relay checkpoints each room's current tip, so it does not
wake Codex for old conversation history. Add `--from-start` only when that
backlog is intentionally actionable. The cursor is stored in `state_db`, bound
to both target alias and room, and advanced only after the recipient
acknowledges the corresponding thin wake event. Until then, lease expiry makes
the relay eligible to actuate the same durable event again. `relay --once`
performs one scan for supervision and tests. The managed loop retries transient
Cowchat and app-server startup failures. A stopped relay cannot provide
low-latency wakes, but permanent Cowchat history is caught up when it restarts.
One failing target is reported without preventing later configured targets from
being scanned. The relay keeps one long-lived multi-room Cowchat connection and
reconnects before backfilling from its durable cursors after a transport loss.

Persisted state has a stable installation scope plus an independent identity
and resettable `state_id` for each target alias. Editing or adding one target
does not reset any other target. Changing one alias's semantic room, recipient,
or Codex thread identity rotates only that alias at the room's verified current
tip, while runtime policy and polling changes preserve its state. To
intentionally rotate one current target (for example, after a coordinated
server-state reset), reset only that target with:

```bash
cowchat-codex --config /absolute/path/to/codex-wake.json \
  reset-state --target reviewer
```

The command first connects to the configured permanent room, validates its
encryption setup, and captures its durable tip. It then rotates the target's
`state_id`, seeds its inbox and relay floor at that tip, and invalidates every
older read, acknowledgement, relay, delivery, and wake operation. Old Cowchat
messages remain retained but their old `state_id` makes them ineligible for the
new inbox. Unbounded read-only recovery scans run without holding a filesystem
lock, then revalidate the target generation at each commit point. A
cross-process asynchronous target fence covers state rotation and each bounded
Cowchat send or Codex actuation, so an old generation cannot commit or begin a
new side effect after reset. The database, SQLite sidecars, and the dedicated
target-lock directory and files are owner-only on Unix. Database paths are
canonicalized into one lock namespace; ambiguous hard-link aliases and unsafe
lock-file replacements fail closed.
`state_db` must name a real filesystem database; SQLite in-memory and `file:`
URI forms are rejected because bridge processes must share durable state.

### Upgrading pre-v0.7 state

The v0.7 bridge refuses to start when a state database contains nonempty v0.6
`wake_events` or `wake_target_state` tables. Those tables did not record the
installation scope, target identity, or exact read cursors, so silently
creating a new target at the room tip could skip a delivered but
unacknowledged wake.

Stop the MCP and relay processes, then migrate each reported alias using the
current configuration:

```bash
cowchat-codex --config /absolute/path/to/codex-wake.json \
  migrate-legacy-state --target reviewer
```

The command resolves the configured permanent room and its live tip, then in
one SQLite transaction binds the alias to the configured target identity and
room, copies its idempotency/event records, preserves its acknowledged cursor,
and removes only that alias's legacy rows. The old aggregate `max_read_seq` is
not trusted because v0.6 did not retain exact returned cursors; pending events
must be read again before acknowledgement. Delivered legacy Cowchat envelopes
remain readable only when their sequence, content, target/source/event
metadata, hint, and migrated local record match exactly.

If legacy state is corrupt or deliberately obsolete and losing its pending
cursor/idempotency state is acceptable, use the explicit reset fallback:

```bash
cowchat-codex --config /absolute/path/to/codex-wake.json \
  reset-state --target reviewer --discard-legacy-state
```

This keeps durable Cowchat history but intentionally makes the discarded wake
generation ineligible and seeds the target at the verified live tip. A normal
`reset-state` does not bypass the legacy-state startup guard. Repeat migration
or explicit discard for every alias named by the error before restarting the
bridge.

## Tool contract

### `wake_agent`

Appends one CloudEvents-shaped event to the configured target's Cowchat room.
The idempotency key is `(target, source, event_id)`. Reusing that key with
different caller-controlled content or a different wake hint is rejected. If
`time` is omitted, the first generated timestamp is persisted and reused by
exact retries.

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

The event is committed before the Codex wake is attempted. A cross-process
delivery claim prevents two bridge processes sharing `state_db` from sending
the same envelope concurrently. If a process crashes after the Cowchat send,
the next claim recovers only a message from a derived bridge role whose exact
target alias, `state_id`, content, hint, and stored digest match the
reservation. The claim is renewed while historical recovery is scanned and
rechecked immediately around delivery, so an expired owner cannot send after a
new process takes over. Failed wake attempts release only the
generation-fenced wake lease they own.

### `wake_inbox_read`

Reads target-addressed events after a Cowchat room sequence. With no
`after_cursor` or `state_id`, reading starts after the current generation's
last acknowledged cursor. An explicit cursor requires the matching `state_id`
returned by an earlier read and must be either the acknowledged cursor or an
exact sequence returned by that generation. An empty read cannot manufacture
new acknowledgement authority. Before returning a later event, the bridge
also verifies that no lower locally delivered wake envelope is missing from
retained Cowchat history. Returned event data is untrusted external input.

### `wake_inbox_ack`

Advances the cursor after processing and requires the `state_id` returned by
`wake_inbox_read`. The bridge rejects a stale generation, an acknowledgement
beyond the highest sequence it returned, or a range containing any delivered
event that was not actually returned by a read. An eligible event that arrived
during the prior wake can trigger one follow-up wake after acknowledgement;
lower-priority events remain durable but cannot bypass the target's
`min_wake_hint` policy.

## Delivery behavior

- Events are at-least-once at the tool boundary and exactly-once in the local
  bridge database for a fixed idempotency key.
- One target has at most one live wake lease. Further events coalesce into the
  same durable inbox instead of starting a turn per event.
- Relay source pages are consumed in exact room-sequence order. Inbox reads
  separately require every locally delivered wake through the returned cursor.
  Missing retained source messages, a room rollback below local state, and a
  missing lower wake envelope all fail closed instead of advancing a cursor.
- An idle or unloaded thread receives `turn/start`. A regular active thread
  receives `turn/steer` with the exact in-progress `expectedTurnId`; the
  app-server rejects a race rather than starting a second turn. Both calls use
  fixed text input, trusted application protocol context, and a separate
  untrusted thin reference. Active review and manual-compaction turns that
  cannot accept direct input are rejected, and only the matching lease
  generation is released.
- The agent, not the bridge, decides when processing is complete by calling
  `wake_inbox_ack`. The managed relay keeps the source message pending until
  that acknowledgement, so a crashed or non-acking turn is retried after the
  wake lease rather than being silently stranded.
- For relay-enabled targets, an ordinary peer message becomes an idempotent
  `cowchat.message.received` event keyed by the original message id. The event
  contains only message and sender references; the original content remains in
  Cowchat and must be read there. This is the supported path for a natural
  follow-up to resume a Codex task after its previous turn ended.

## Current limitations

- Target aliases, thread ids, and rooms are static configuration; there is no
  authorization-minting UI yet.
- The MCP, relay, and doctor roles may operate concurrently only when they
  share the same `state_db`; delivery and wake claims are coordinated there.
  Run at most one live process for each derived role ID, because a second
  same-role Cowchat connection is a reconnect takeover. All live processes for
  one target alias must also use the same semantic target configuration; the
  state fence cannot decide which of two concurrently running configurations
  is newer. Do not copy a live database between endpoint/config identities.
- Cowchat message history is scanned to repair the narrow crash window between
  room commit and local delivery bookkeeping. Large historical rooms may need
  a server-side metadata lookup before this is a production-scale receiver.
- Codex app-server's `additionalContext` API is experimental and may change.
- The relay is an operator-run local service, not a server-side global feature.
  Without it (or an explicit `wake_agent` call), ordinary Cowchat messages do
  not restart an ended Codex task; a returning `wait` works only while the task
  remains active.
- Full Agent Wake Protocol conformance still requires the signed HTTP receiver,
  replay protection, scoped capability budgets, revocation, and conformance
  vectors described by that protocol.
