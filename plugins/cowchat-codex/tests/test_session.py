import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import uuid


SCRIPT = Path(__file__).resolve().parents[1] / "hooks" / "session.py"
spec = importlib.util.spec_from_file_location("cowchat_codex_hook", SCRIPT)
hook = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hook)


class HookTests(unittest.TestCase):
    def test_installed_app_cli_is_used_before_path(self):
        with tempfile.TemporaryDirectory() as root:
            home = Path(root)
            config = home / ".cowchat/app-cli-binary"
            config.parent.mkdir()
            cli = home / "Cowchat.app/Contents/Helpers/cowchat"
            cli.parent.mkdir(parents=True)
            cli.write_text("#!/bin/sh\n")
            cli.chmod(0o700)
            config.write_text(str(cli))
            with patch.object(hook.Path, "home", return_value=home):
                self.assertEqual(hook.cowchat_binary(), str(cli))
                cli.unlink()
                self.assertEqual(hook.cowchat_binary(), "cowchat")

    def test_start_and_resume_keep_private_identity_out_of_context(self):
        with tempfile.TemporaryDirectory() as root:
            home = Path(root)
            (home / ".cowchat").mkdir()
            (home / ".cowchat/hosted-bot.json").write_text(json.dumps({"url": "wss://chat.example/ws"}))
            identifier = str(uuid.uuid4())
            payload = {"session_id": identifier, "cwd": root}
            calls = []

            def fake_run(args, session_file=None):
                calls.append((args, session_file))
                return subprocess.CompletedProcess(args, 0, json.dumps({
                    "room_id": "room-1", "room_name": "test-room",
                    "session_file": str(home / ".cowchat/sessions/private.json"),
                }), "")

            with patch.object(hook.Path, "home", return_value=home), patch.object(hook, "run", side_effect=fake_run), patch.object(hook, "start_listener"), patch("sys.stdout", new_callable=io.StringIO) as output:
                hook.hook_main("start", payload)
                context = json.loads(output.getvalue())["hookSpecificOutput"]["additionalContext"]
            mapping_dir = home / ".cowchat/codex-sessions"
            mappings = list(mapping_dir.glob("*.json"))
            self.assertEqual(len(mappings), 1)
            record = json.loads(mappings[0].read_text())
            self.assertEqual(record["id"], calls[0][0][calls[0][0].index("--id") + 1])
            self.assertNotIn(record["id"], context)
            self.assertNotIn(record["session_file"], context)
            self.assertNotIn(identifier, mappings[0].name)
            self.assertEqual(os.stat(mappings[0]).st_mode & 0o077, 0)
            Path(record["session_file"]).parent.mkdir()
            Path(record["session_file"]).write_text("private")
            os.chmod(record["session_file"], 0o600)
            with patch.object(hook.Path, "home", return_value=home), patch.object(hook, "run", side_effect=fake_run), patch.object(hook, "start_listener"), patch("sys.stdout", new_callable=io.StringIO):
                hook.hook_main("start", payload)
            self.assertEqual(calls[1][1], record["session_file"])
            self.assertEqual(calls[1][0][calls[1][0].index("--id") + 1], record["id"])
            with patch.object(hook.Path, "home", return_value=home), patch.object(hook, "run", side_effect=fake_run):
                for event in ("working", "prompt", "idle", "end"):
                    hook.hook_main(event, payload)
            for heartbeat, presence in ((2, 3), (4, 5), (6, 7)):
                self.assertEqual(calls[heartbeat][0][-3:], ["session", "heartbeat", record["id"]])
                self.assertEqual(calls[presence][0][-4:-2], ["presence", ("working", "prompt", "idle")[(heartbeat - 2) // 2]])
            self.assertEqual(calls[8][0][-3:], ["session", "end", record["id"]])
            self.assertFalse(mappings[0].exists())


if __name__ == "__main__":
    unittest.main()
