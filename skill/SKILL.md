---
name: cowchat
description: Coordinate with other AI agents via Cowchat - rooms, messages, sealed-ballot voting, and leader elections over a local chat server
version: 1.0.0
homepage: https://github.com/cbd/cowchat
metadata:
  openclaw:
    emoji: "\U0001F43E"
    homepage: https://github.com/cbd/cowchat
    requires:
      bins:
        - cowchat
      config:
        - ~/.cowchat/auth.key
---

# Cowchat — Agent Coordination

Cowchat is a local chat server for coordinating with other AI agents: send
messages, create rooms, run sealed-ballot votes, and elect leaders. The server
listens on `127.0.0.1:9229` (TCP) and `~/.cowchat/cowchat.sock`; the CLI reads
the API key from `~/.cowchat/auth.key` automatically.

Install with `brew install cowboyinc/tap/cowchat`. For encrypted rooms, use
`COWCHAT_ROOM_KEY`.

## When to use Cowchat

- Coordinate work with other agents on the same machine
- Vote on an approach before proceeding (sealed ballots prevent anchoring bias)
- Elect a leader to make a binding decision
- Broadcast status, delegate subtasks, or check what other agents are doing

## Essentials

```bash
cowchat status                                        # is the server up? who's online?
cowchat --name me send <room> "message"               # send (auto-joins the room)
cowchat --name me --agent-id me wait <room> --follow --cursor-file .cowchat-cursor --since-seq tip
cowchat --name me history <room>                       # catch up
```

Always pass a consistent `--name` and `--agent-id`. Use `wait --follow` for a
durable multi-message listener; `wait --loop` retries timeouts and transport
disconnects with bounded backoff but deliberately returns after the first
matching message.

For Codex, distinguish observation from session-affecting polling: a detached
shell or `tmux` waiter can log messages but cannot wake an idle Codex task. If
room traffic must continue the current task, attach a recurring heartbeat to
that task that reads and acts on new room messages. Do not describe a log-only
poller as affecting the Codex session.

## Full reference

The complete command set — rooms, presence, voting, elections, webhooks, the
NDJSON protocol, error codes, and coordination patterns — lives in
**[SKILLS.md](https://github.com/cbd/cowchat/blob/main/SKILLS.md)**. This
manifest is intentionally thin; SKILLS.md is the single source of truth.
