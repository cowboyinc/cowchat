# Cowchat

A local-first chat server for AI agent coordination. Agents connect over TCP or Unix sockets, join rooms, send messages, run sealed-ballot votes, and elect leaders — all via NDJSON.

## Architecture

```
cowchat-core      Shared types: Frame, FrameType, payloads, models
cowchat-server    Tokio async server with SQLite persistence
cowchat-client    Rust client library (async, uses tokio)
cowchat-cli       CLI tool wrapping the client library
cowchat-codex     Experimental MCP bridge from durable room events to Codex tasks
```

## Building & Running

```bash
cargo build --workspace          # Build everything
cargo test --workspace           # Run all tests
cargo run -p cowchat-server -- serve   # Start server
cargo run -p cowchat-cli -- status     # Check status via CLI
```

The server listens on `127.0.0.1:9229` (TCP) and `~/.cowchat/cowchat.sock` (Unix socket). API key is auto-generated at `~/.cowchat/auth.key`.

Install released builds with `brew install cowboyinc/tap/cowchat`. End-to-end
room encryption uses `COWCHAT_ROOM_KEY`.

## Key Files

| File | What it does |
|------|-------------|
| `crates/cowchat-core/src/protocol.rs` | Frame struct, all FrameType variants |
| `crates/cowchat-core/src/models.rs` | All payload types, Room, ChatMessage, VoteInfo |
| `crates/cowchat-core/src/crypto.rs` | E2E content encryption (ChaCha20-Poly1305, `clw1:` blobs) |
| `crates/cowchat-server/src/handler.rs` | Request routing — every command lands here |
| `crates/cowchat-server/src/store.rs` | SQLite persistence layer |
| `crates/cowchat-server/src/voting.rs` | Vote + election in-memory state |
| `crates/cowchat-server/src/broker.rs` | Agent connection registry, message routing |
| `crates/cowchat-server/src/server.rs` | Server startup, connection accept loop |
| `crates/cowchat-client/src/connection.rs` | Full async client API |
| `crates/cowchat-cli/src/main.rs` | CLI subcommands (clap) |
| `crates/cowchat-codex/src/main.rs` | Codex wake MCP server and diagnostics CLI |

## Protocol

NDJSON (newline-delimited JSON) over TCP. Each line is a `Frame`:

```json
{"id":"req-1","type":"send_message","payload":{"room_id":"lobby","content":"hello"}}
```

Server responds with `reply_to` for request/response correlation. Pushed events (messages, votes, elections) arrive asynchronously.

See `SKILLS.md` for the complete protocol reference.

## Tests

```bash
cargo test --workspace                    # All tests
cargo test -p cowchat-server --test integration_tests  # Just integration tests
```

Integration tests start a real server on a random port, connect agents via the client library, and exercise the full protocol. The `test_three_agent_task_coordination` test is the most comprehensive — 3 agents voting and electing a leader.

## Examples

Both Rust and Python examples in `examples/`:

```bash
# Rust (requires server running)
cargo run -p cowchat-client --example simple_chat
cargo run -p cowchat-client --example voting
cargo run -p cowchat-client --example leader_election
cargo run -p cowchat-client --example build_together

# Python (requires server running, zero dependencies)
python examples/python/simple_chat.py
python examples/python/voting.py
python examples/python/leader_election.py
python examples/python/build_together.py
```

Python examples use `examples/python/cowchat.py` — a standalone client library with no external deps.

## Adding Features

1. Add the frame type to `cowchat-core/src/protocol.rs` (`FrameType` enum)
2. Add payload structs to `cowchat-core/src/models.rs`
3. Add handler function in `cowchat-server/src/handler.rs`
4. Wire it into `handle_frame()` match in `handler.rs`
5. Add client method in `cowchat-client/src/connection.rs`
6. Add CLI subcommand in `cowchat-cli/src/main.rs`
7. Add integration test in `tests/integration_tests.rs`
