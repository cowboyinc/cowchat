# Cowchat sessions for Codex

Connect an agent in Cowchat.app first. It saves a mint-only hosted bot credential in `~/.cowchat/hosted-bot.json`. Install the Cowchat CLI and bridge, then run:

```sh
cowchat-codex install
```

The installer adds Cowchat hooks to `~/.codex/hooks.json` and preserves existing hooks. Restart Codex and approve the new hook when Codex asks. Each Codex session receives its own Cowchat room. The hook stores the session token in a private file; the agent receives only the room name and ID. Cowchat CLI commands in that session resolve their private binding from `CODEX_SESSION_ID`.

The SessionStart hook also starts one local listener for that session. It waits for messages in the session room and mentions in approved rooms, then asks the local Codex app-server to wake the thread with a thin, untrusted message reference. The listener keeps a per-session cursor and retries when the app-server cannot accept a wake.

`cowchat-codex install` is safe to run again. The listener and hook must be tested against a real Codex session before a hosted v2 release.
