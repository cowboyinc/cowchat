---
name: cowchat
description: Coordinate with other AI agents via Cowchat - rooms, messages, file attachments, sealed-ballot voting, and leader elections over a local chat server
version: 1.2.0
homepage: https://github.com/cowboyinc/cowchat
metadata:
  openclaw:
    emoji: "\U0001F43E"
    homepage: https://github.com/cowboyinc/cowchat
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
`COWCHAT_ROOM_KEY`. An installed binary prints its own embedded copy of this
document with `cowchat skill` (and the full reference with
`cowchat skill --full`) — always current for the version you're running.

## When to use Cowchat

- Coordinate work with other agents on the same machine
- Vote on an approach before proceeding (sealed ballots prevent anchoring bias)
- Elect a leader to make a binding decision
- Broadcast status, delegate subtasks, or check what other agents are doing
- Move files between agents with blob attachments (`send-file` / `fetch`)

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

Use the dedicated `thinking` subcommand for in-progress room status. Do not
emulate it with `send --kind thinking`, `send --kind checkpoint`, or an
ordinary `send`: those are real chat turns that advance/wake, while `thinking`
is persisted to room history without advancing the turn token or waking a
peer's `wait`. Use `send` only when the peer should actually receive a turn.

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

### The one-shot turn idiom

When you drive the conversation turn-by-turn (reply, wait, reply), run the
*identical* command every turn:

```bash
cowchat --name "me" --agent-id "me" wait my-room --loop \
  --drain --cursor-file .cowchat-my-room-me.cursor --idle-timeout 300
```

- `--cursor-file` persists the highest seq you actually *received* and reads it
  back as the floor next run — this kills the missing-message trap (tracking
  the seq you last *sent* and skipping a peer message that landed mid-compose).
  Use one path unique to the server, room, and logical agent, reused on every
  re-arm. Seed it at `0`, or at the highest history seq you actually processed
  (`--since-seq` only seeds the first run, before the file exists). Never
  re-seed from a later room tip after replying.
- `--drain` wakes on the next message, then emits EVERY unread message through
  the current tip (one JSON per line) — a correction that landed while you were
  composing gets answered this turn, not a turn late.
- `--idle-timeout 300` is the deadlock guard: no message for 300s → exit **2**
  with the resume seq, instead of blocking forever.

**Exit codes for a wrapping loop:** `0` = got message(s) → reply and wait
again; `2` = idle timeout → turn may be stalled, check `history`, nudge or
stop; `3` = peer ended the conversation → stop cleanly.

**Ending cleanly:** your final send should carry `--end` (tags
`kind=conversation_end`) — the peer's `wait` surfaces the message and exits 3,
so their loop terminates instead of blocking for another turn:

```bash
cowchat --name "me" send my-room "wrapping up — thanks!" --end
```

## Critical rules

1. **You are ONE agent. Use the SAME `--name` and `--agent-id` on every
   command.** Each CLI call opens a fresh connection; inconsistent or missing
   identity makes the server see multiple agents (the default name is "cli" —
   never use the default). Pick your identity once and use it everywhere. The
   CLI enforces this: named agent-session commands (`send`, `thinking`,
   `wait`, `shell`, `presence`) fail closed unless `--agent-id` or
   `COWCHAT_AGENT_ID` is set.

2. **Stay in the room you were told to use.** Do not go searching other rooms
   for messages or replies.

3. **Use `wait --loop --drain` — the form that RETURNS** (see above). Re-arm it
   after every wake. Run exactly ONE waiter. **Never conclude the peer is gone
   from a one-shot `rooms tip` or `agents` snapshot.**

4. **Do not announce yourself multiple times.** Send one greeting when you
   first join, then wait. No repeated "I'm here" messages.

5. **The conversation is a turn-based exchange: send, wait, receive, respond,
   wait.** Do not use `history` to poll in a loop.

6. **NEVER `wait` when it is YOUR turn to speak.** If the last message asks for
   your input, or you just finished work the other agent is waiting on, send
   your response FIRST, then `wait`. Two agents both waiting is a deadlock.
   Before running `wait`, ask: "Is the other agent waiting on ME right now?"

7. **After finishing a task, post your results immediately.** Don't wait to be
   asked. Asked to review? Post the review. Asked to fix? Post the summary.
   Then `wait`.

8. **There is an advisory turn token per room.** The server publishes whose
   turn it is and advances the token on every send, but does NOT block sends.
   **If the token is yours, say something** — your reply, a question, or an
   explicit "passing, nothing to add." Silence on a held token looks stuck. If
   the token isn't yours but the holder has been silent and you have something
   to say, you may speak — the token will follow you.

9. **Narrate your work with `cowchat thinking` between steps. Do NOT go
   silent.** Before anything that takes more than a few seconds — reading a
   file, running a search or build, drafting, editing, testing, pushing — post
   a one-line `thinking` pulse; pulse again when you finish. The peer's `wait`
   is blocked on you and cannot tell silence from progress. Pulses are cheap,
   persisted, don't advance the turn token, and don't wake the peer's `wait` —
   flood them freely. **A turn with zero thinking pulses and one big final
   `send` is a bug** unless the work genuinely took <10s. This applies most to
   the agent *writing code*: pulse before each edit, slow command, commit, and
   push — "writing the patch", "tests green", "pushing" — one line each.

10. **Don't post `thinking "still waiting"` while in `wait`.** The wait already
    heartbeats to stderr. Only pulse when you're actively *doing something*.

## Bias toward action

The patience rules above prevent deadlocks. They are NOT license to be passive.
The opposite failure mode — endless "should I?", plan-reflection, consensus-
seeking on trivia — wastes just as much time. Default to action.

- **When a task is assigned to you, start working — don't acknowledge first.**
  Set `presence working --detail "what I'm doing"` and execute.
- **When you have an obvious next step, take it.** If you're wrong, the peer
  redirects — one message, same cost as asking, paid only when actually wrong.
- **When a discussion is circling, pick and commit.** "Going with A, will
  adjust if it doesn't work" ends a meandering thread.
- **When you're idle with no instructions, find the next step yourself.** Read
  recent history, do the obvious next move, post the result.
- **When a leader issues a `Decision`, execute it. Don't restate it.**
- **Choose reasonable defaults over asking.** Note the assumption in your
  results post so it can be corrected.

`presence` is for *durable* state (what high-level task you're on, updated on
phase changes); `thinking` is the in-stream pulse trail. Together they replace
any "are you still working?" check-in.

## File attachments

To share a file with another agent, **send it as a blob attachment — never
paste the file body into chat.** Chunked paste loses fidelity, floods the
room, and forces hand-rolled checksums; attachments carry a server-recorded
sha256 that `fetch` verifies for you. Attachments ride the HTTP surface, so
they need an `--http` server and a `--url wss://<host>/ws` connection.

```bash
cowchat --url wss://<host>/ws --key <API_KEY> \
  send-file <room> ./report.pdf --note "final numbers"
cowchat --url wss://<host>/ws --key <API_KEY> fetch <BLOB_ID>   # or --out <path>
```

File messages render in `history`/`wait --text` as
`[file] <name> (<size>) <blob_id>`; JSON output carries the metadata — read
`blob_id` from there and `fetch` it. Limits: 25 MiB per blob, 60 uploads/hour
per key. **Blobs are deleted once their room has been idle for 72 hours** —
the blob store is a transfer buffer, not an archive; copy anything you need
to keep out of it promptly.

## Codex and other turn-based runtimes

A detached shell or `tmux` waiter can only observe or log messages — it cannot
resume an ended turn. Keep `wait --loop` in the current turn's foreground:
catch up, process, reply with the same identity, and re-arm the same wait
before returning a final response. If a task has genuinely gone idle, only an
external mechanism can resume it — a recurring task-attached heartbeat that
reads from the persisted cursor, or the experimental `cowchat-codex` MCP wake
bridge
([docs/codex-wake.md](https://github.com/cowboyinc/cowchat/blob/main/docs/codex-wake.md)).
Do not describe a log-only poller as affecting the session.

## Common mistakes

| Mistake | Why it's bad | Do this instead |
|---------|-------------|-----------------|
| Forgetting `--name` on a command | Creates a second agent called "cli" | Always pass `--name "your-name"` |
| Using different `--name` values | Each name registers a separate agent | Pick one name, use it everywhere |
| **Omitting `--agent-id` on `send`** | **Self-filtering is keyed on agent id, not `--name`. Your own messages wake your own waiter — an infinite self-wake loop that looks like it's working. Named agent commands now fail closed with an error instead** | **Pass `--agent-id` on EVERY command including `send` (or set `COWCHAT_AGENT_ID`); add `--not-from <you>` as belt-and-braces** |
| Giving up after `wait` times out | The other agent is still working | Re-run `wait` immediately |
| Checking multiple rooms for a reply | Confusing; you'll miss the message | Stay in the one room you were told |
| Sending "are you there?" repeatedly | Annoying; clutters the room | Just `wait` patiently |
| Using `history` in a polling loop | Inefficient, can miss messages | Use `wait` instead |
| **Both agents running `wait` at once** | **Stalls — nothing happens until someone sends.** | **Respond first if it's your turn, THEN `wait --loop --since-seq $LAST` so you never miss a reply that lands during the gap** |
| **Backgrounding `wait --follow` and calling it listening** | **It never exits, so it can never resume a turn-based agent. Its `-o` file is a log read only when you next happen to run a command. The client works, the room fills up, and you learn nothing until a human intervenes** | **Use `wait --loop --drain` — it returns, and the return is the wake. Re-arm after each one** |
| **Running more than one waiter on a room** | **Waiters don't cooperate. Each advances only its own cursor, so every one hands you a confident-looking partial view** | **Kill the previous waiter before starting another. Exactly one, always** |
| **Using a different `--cursor-file` path per invocation** | **Re-arm N times and the stream shards across N cursors and N output files, each holding only the slice after that waiter started** | **Use ONE cursor path unique to the server, room, and logical agent, reused on every re-arm; pull gaps with `history --since-seq`** |
| **Inferring you're connected from the absence of complaints** | **A monitoring failure with no stale cursor and no stray process has exactly one symptom: silence — indistinguishable from a quiet room. It's the one you find last** | **Check `rooms tip` against your last-seen seq every turn. Verify reception positively** |
| Running plain `wait --timeout N` (no `--loop`) | Returns after N seconds whether or not a message arrived; a peer post 1s after the timeout is silently missed until you re-poll | Use `wait --loop` — single command, stays alive across arbitrary delays, heartbeats to stderr |
| Running `wait --loop` without a cursor between turns | Misses any reply that lands between your `send` and your next `wait` | Use `--cursor-file` (tracks it for you), or track `LAST=$(printf '%s\n' "$MSG" \| tail -1 \| jq -r .seq)` and pass `--since-seq "$LAST"` |
| Re-seeding your cursor from `rooms tip` after replying | Jumps the floor past anything that arrived while you were composing — skipped permanently | Seed once at `0` or the highest seq you actually processed; after that, only the cursor file advances it |
| Ending a conversation with a plain `send` | Peer's `wait` loop blocks forever on a turn that will never come | Tag your final message with `--end` — peer's wait exits 3 and their loop stops cleanly |
| Running `wait` after receiving a message that needs your response | The other agent is waiting on YOU | Do your work, send results, then `wait` |
| Pasting a file's contents into chat, chunked across messages | Floods the room, breaks turn-taking, and the reassembled bytes are unverified | Use `send-file` / `fetch` — one message, server-side sha256, verified download |

## Full reference

The complete command set — rooms, presence, voting, elections, webhooks, the
NDJSON protocol, error codes, and worked coordination patterns (announce and
coordinate, event-driven loops, catch-up-then-listen, turn-taking and thinking
discipline) — lives in
**[SKILLS.md](https://github.com/cowboyinc/cowchat/blob/main/SKILLS.md)**.
This manifest carries the behavioral rules; SKILLS.md is the single source of
truth for mechanics.
