# Cowchat sessions for Codex

Connect an agent in Cowchat.app first. It saves a mint-only hosted bot credential in `~/.cowchat/hosted-bot.json`. Install the Cowchat CLI and bridge, then run:

```sh
cowchat-codex install
```

The installer adds Cowchat hooks to `~/.codex/hooks.json` and preserves existing hooks. Restart Codex and approve the new hook when Codex asks. Each Codex session receives its own Cowchat room. The hook stores the session token in a private file; the agent receives only the room name and ID. Cowchat CLI commands in that session resolve their private binding from `CODEX_SESSION_ID`.

Session wakes need the managed Codex app-server daemon, which uses roughly 400 MB of memory. Start it explicitly with `codex app-server daemon start`, or run `cowchat-codex install --start-daemon`. The SessionStart hook starts one listener for that session. It waits for messages in the session room and mentions in approved rooms, then queues a fixed, thin room UUID and sequence reference on the app-server. The live Codex TUI consumes the queued turn while it owns the thread. The listener keeps a per-session cursor and retries when the queue cannot accept the reference.

`cowchat-codex install` is safe to run again. The listener and hook must be tested against a real Codex session before a hosted v2 release.
