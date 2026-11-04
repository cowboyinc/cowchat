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
                "if 'start' in sys.argv: print(json.dumps({'session_file':'/private/session.json','room_id':'room-1','room_name':'work'}))\n"
            )
            binary.chmod(0o755)
            calls = home / "calls.jsonl"
            environment = os.environ.copy()
            environment.update(HOME=str(home), PATH=f"{home}:{environment['PATH']}", CALLS=str(calls))
            identifier = str(uuid.uuid4())
            payload = json.dumps({"session_id": identifier, "cwd": str(home)})

            def event(name):
                return subprocess.run(
                    [sys.executable, str(ROOT / "hooks" / "session.py"), name],
                    input=payload, text=True, capture_output=True, env=environment, check=True,
                )

            started = event("start")
            context = json.loads(started.stdout)["hookSpecificOutput"]["additionalContext"]
            self.assertIn("COWCHAT_SESSION_FILE=/private/session.json", context)
            self.assertNotIn("mint-secret", context)
            event("start")
            event("working")
            event("prompt")
            event("idle")
            event("end")
            invocations = [json.loads(line) for line in calls.read_text().splitlines()]
            self.assertIsNone(invocations[0]["file"])
            self.assertTrue(all(call["file"] == "/private/session.json" for call in invocations[1:]))
            self.assertFalse((home / ".cowchat" / "claude-sessions" / f"{identifier}.json").exists())


if __name__ == "__main__":
    unittest.main()
