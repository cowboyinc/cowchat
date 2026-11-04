#!/usr/bin/env python3
"""Claude Code hook for a hook-bound Cowchat session.

The bot mint credential remains in the Cowchat CLI's private config. The hook
stores only the CLI's opaque session-file path, never its token.
"""

import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import uuid


def run(args, session_file=None):
    env = os.environ.copy()
    if session_file:
        env["COWCHAT_SESSION_FILE"] = session_file
    return subprocess.run(args, env=env, text=True, capture_output=True, timeout=15, check=True)


def session_id(value):
    return str(uuid.UUID(value))


def hook_main(event, payload):
    identifier = session_id(payload["session_id"])
    home = Path.home() / ".cowchat"
    hosted = json.loads((home / "hosted-bot.json").read_text())
    url = hosted["url"]
    if not url.startswith("wss://") and not re.match(r"^ws://(127\.0\.0\.1|localhost|\[::1\])(?::|/)", url):
        raise ValueError("Cowchat session server must use WSS or loopback WS")
    records = home / "claude-sessions"
    records.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(records, 0o700)
    mapping = records / f"{identifier}.json"
    saved = json.loads(mapping.read_text()) if mapping.exists() else None
    agent_id = f"session:{identifier}"
    common = ["cowchat", "--url", url, "--agent-id", agent_id]

    if event == "start":
        cwd = Path(payload.get("cwd") or os.getcwd())
        folder = cwd.name or "workspace"
        branch = subprocess.run(
            ["git", "-C", str(cwd), "branch", "--show-current"],
            text=True, capture_output=True, timeout=3, check=False,
        ).stdout.strip() or "main"
        name = saved["name"] if saved else f"Claude · {folder}"
        if saved:
            folder, branch = saved["folder"], saved["branch"]
        start = ["cowchat", "--url", url, "--name", name, "--agent-id", agent_id] + [
            "session", "start", "--id", agent_id, "--harness", "claude",
            "--folder", folder, "--branch", branch,
        ]
        result = json.loads(run(start, saved["session_file"] if saved else None).stdout)
        session_file = result["session_file"]
        record = {"name": name, "folder": folder, "branch": branch, "session_file": session_file}
        temporary = mapping.with_suffix(".tmp")
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w") as stream:
            json.dump(record, stream)
        os.replace(temporary, mapping)
        cursor = records / f"{identifier}.cursor"
        command = " ".join(shlex.quote(part) for part in [
            "cowchat", "--url", url, "--agent-id", agent_id,
            "wait", result["room_id"], "--approved-rooms", "--mentions-only",
            "--loop", "--drain", "--heartbeat-secs", "60",
            "--cursor-file", str(cursor),
        ])
        command = f"COWCHAT_SESSION_FILE={shlex.quote(session_file)} {command}"
        context = (
            f"Your Cowchat session room is {result['room_name']} ({result['room_id']}). "
            "Read the Cowchat skill and the latest 50 messages first; search older history only when needed. "
            "Use Cowchat to collaborate with agents in this room. When idle, run "
            f"this returning wait command and repeat it after each reply: {command}. "
            "Use the same COWCHAT_SESSION_FILE, --url and --agent-id for Cowchat send and presence."
        )
        print(json.dumps({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": context}}))
    elif saved:
        env_file = saved["session_file"]
        if event == "end":
            run(common + ["session", "end", "--id", agent_id], env_file)
            mapping.unlink(missing_ok=True)
        elif event in ("working", "idle", "prompt"):
            run(common + ["session", "heartbeat", "--id", agent_id], env_file)
            run(common + ["presence", event, "--detail", "Claude Code session"], env_file)


if __name__ == "__main__":
    try:
        hook_main(sys.argv[1], json.load(sys.stdin))
    except (KeyError, ValueError, OSError, subprocess.SubprocessError) as error:
        print(f"Cowchat hook: {error}", file=sys.stderr)
        sys.exit(0)
