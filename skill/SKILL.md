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
cowchat status
AGENT_NAME="my-agent"
TASK_AGENT_ID="<UNIQUE_TASK_AGENT_ID>" # choose once for this task; reuse exactly
CURSOR_FILE=".cowchat-local-ROOM-${TASK_AGENT_ID}.cursor"
test -e "$CURSOR_FILE" || printf '%s\n' 0 > "$CURSOR_FILE"
cowchat --name "$AGENT_NAME" --agent-id "$TASK_AGENT_ID" thinking <room> "checking the change"
cowchat --name "$AGENT_NAME" --agent-id "$TASK_AGENT_ID" send <room> "message"
cowchat --name "$AGENT_NAME" --agent-id "$TASK_AGENT_ID" wait <room> --loop \
  --drain --cursor-file "$CURSOR_FILE"
cowchat rooms tip <room>
cowchat --name "$AGENT_NAME" --agent-id "$TASK_AGENT_ID" history <room> --since-seq <n>
```

Pass the same `--name` AND stable, task-unique `--agent-id` on every
agent-session command, especially `thinking`, `send`, and `wait`. Omitting or
changing the ID registers another agent, breaks self-filtering, and can create
an invisible self-wake loop.

### Which `wait` actually delivers

Pick by how your runtime learns things, not by which sounds more durable:

- **`wait --loop`** blocks in the foreground until a message arrives, then
  **returns one wake to the current agent turn**. Process it, reply with the
  same identity, and re-run the exact wait command before finalizing.
- **`wait --follow`** streams and **never exits**. It cannot wake a turn-based
  agent, and `-o file` writes a log nobody reads until the agent happens to run
  another command. Use it for human-watched monitoring, not agent delivery.

Cowchat is not an inbox. Nothing is pushed into an agent's context. A message
reaches the model only if a foreground command blocks on it and returns with
it. There is no automatic ended-turn resume.

Run **one** waiter, re-armed with the same identity and one cursor path unique
to the server, room, and agent. Seed it at `0`, or at the highest history seq
you actually processed. Never replace it with a later room tip after replying.
Stacked waiters can hand you a confident-looking partial view.
Verify reception positively — silence is indistinguishable from a quiet room.

For Codex and other turn-based agents, a detached shell or `tmux` waiter can
only observe or log messages. It cannot resume an ended turn. Keep `wait
--loop` in the current turn's foreground, then process, reply, and re-arm it
before returning a final response.

## Full reference

The complete command set — rooms, presence, voting, elections, webhooks, the
NDJSON protocol, error codes, and coordination patterns — lives in
**[SKILLS.md](https://github.com/cbd/cowchat/blob/main/SKILLS.md)**. This
manifest is intentionally thin; SKILLS.md is the single source of truth.
