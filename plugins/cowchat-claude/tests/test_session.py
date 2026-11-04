import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import uuid


ROOT = Path(__file__).resolve().parents[1]


class SessionHookTest(unittest.TestCase):
    def test_start_resume_presence_and_end_keep_the_same_bound_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            (home / ".cowchat").mkdir()
            (home / ".cowchat" / "hosted-bot.json").write_text(
                json.dumps({"url": "wss://chat.example/ws", "api_key": "mint-secret"})
            )
            binary = home / "cowchat"
            binary.write_text(
                "#!/usr/bin/env python3\n"
                "import json,os,sys\n"
                "with open(os.environ['CALLS'], 'a') as out:\n"
                "  out.write(json.dumps({'args':sys.argv[1:],'file':os.getenv('COWCHAT_SESSION_FILE')})+'\\n')\n"
                "if 'start' in sys.argv and os.getenv('COWCHAT_SESSION_FILE') and os.getenv('FAIL_RESUME'):\n"
                "  print('session heartbeat failed: 404 Not Found',file=sys.stderr); sys.exit(1)\n"
                "if 'start' in sys.argv: print(json.dumps({'session_file':os.environ['MOCK_SESSION_FILE'],'room_id':'room-1','room_name':'work'}))\n"
            )
            binary.chmod(0o755)
            calls = home / "calls.jsonl"
            env_file = home / "claude-env"
            session_file = home / "session-token.json"
            session_file.write_text("private token fixture")
            environment = os.environ.copy()
            environment.update(HOME=str(home), PATH=f"{home}:{environment['PATH']}", CALLS=str(calls), CLAUDE_ENV_FILE=str(env_file), MOCK_SESSION_FILE=str(session_file))
            identifier = str(uuid.uuid4())
            payload = json.dumps({"session_id": identifier, "cwd": str(home)})

            def event(name):
                return subprocess.run(
                    [sys.executable, str(ROOT / "hooks" / "session.py"), name],
                    input=payload, text=True, capture_output=True, env=environment, check=True,
                )

            started = event("start")
            context = json.loads(started.stdout)["hookSpecificOutput"]["additionalContext"]
            self.assertNotIn(str(session_file), context)
            self.assertNotIn("mint-secret", context)
            self.assertIn(f"export COWCHAT_SESSION_FILE={session_file}", env_file.read_text())
            self.assertNotIn(f"session:{identifier}", env_file.read_text())
            event("start")
            event("working")
            event("prompt")
            event("idle")
            event("end")
            invocations = [json.loads(line) for line in calls.read_text().splitlines()]
            self.assertIsNone(invocations[0]["file"])
            self.assertTrue(all(call["file"] == str(session_file) for call in invocations[1:]))
            self.assertFalse(list((home / ".cowchat" / "claude-sessions").glob("*.json")))

    def test_expired_resume_mints_a_new_cowchat_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            (home / ".cowchat").mkdir()
            (home / ".cowchat" / "hosted-bot.json").write_text(json.dumps({"url": "wss://chat.example/ws"}))
            binary = home / "cowchat"
            binary.write_text(
                "#!/usr/bin/env python3\n"
                "import json,os,sys\n"
                "with open(os.environ['CALLS'],'a') as out: out.write(json.dumps(sys.argv[1:])+'\\n')\n"
                "if os.getenv('COWCHAT_SESSION_FILE'): print('session heartbeat failed: 401 Unauthorized',file=sys.stderr); sys.exit(1)\n"
                "print(json.dumps({'session_file':os.environ['MOCK_SESSION_FILE'],'room_id':'r','room_name':'work'}))\n"
            )
            binary.chmod(0o755)
            token = home / "token.json"
            token.write_text("fixture")
            calls = home / "calls"
            env_file = home / "env"
            env = os.environ.copy()
            env.update(HOME=str(home), PATH=f"{home}:{env['PATH']}", CALLS=str(calls), MOCK_SESSION_FILE=str(token), CLAUDE_ENV_FILE=str(env_file))
            payload = json.dumps({"session_id": str(uuid.uuid4()), "cwd": str(home)})
            for _ in range(2):
                subprocess.run([sys.executable, str(ROOT / "hooks" / "session.py"), "start"], input=payload, text=True, capture_output=True, env=env, check=True)
            starts = [json.loads(line) for line in calls.read_text().splitlines()]
            ids = [args[args.index("--id") + 1] for args in starts]
            self.assertEqual(len(ids), 3)
            self.assertEqual(ids[0], ids[1])
            self.assertNotEqual(ids[1], ids[2])


if __name__ == "__main__":
    unittest.main()
