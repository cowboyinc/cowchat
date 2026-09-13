#!/usr/bin/env python3
"""Compose the real local room worker, gateway dispatch and harness wake handler.

Requires the three explicitly test-only fixture example binaries. No live keys,
providers, node, consensus calls, or production serving flags are involved.
"""
import argparse
import base64
import json
import select
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("room", "gateway", "harness"):
        parser.add_argument(f"--{name}-bin", required=True, type=Path)
    args = parser.parse_args()
    state = Path(tempfile.mkdtemp(prefix="cowchat-m-echo-"))
    processes = []
    logs = []

    def port():
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            return listener.getsockname()[1]

    room_bind, gateway_bind, harness_bind = [f"127.0.0.1:{port()}" for _ in range(3)]
    room_origin = f"http://{room_bind}"
    harness_origin = f"http://{harness_bind}"
    gateway_url = f"http://{gateway_bind}/0x{'09' * 20}/wake"

    def start(label, command, field):
        stderr = (state / f"{label}.stderr").open("wb")
        logs.append(stderr)
        process = subprocess.Popen([str(v) for v in command], stdout=subprocess.PIPE, stderr=stderr)
        processes.append(process)
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError(f"{label} exited before ready: {process.returncode}")
            if select.select([process.stdout], [], [], 0.2)[0]:
                line = process.stdout.readline()
                try:
                    item = json.loads(line)
                except (ValueError, UnicodeDecodeError):
                    continue
                if field in item:
                    return process
        raise RuntimeError(f"{label} readiness timeout")

    def room_start(label):
        return start(label, [args.room_bin, "serve", room_bind, state], "room_origin")

    def harness_start(label):
        return start(label, [args.harness_bin, harness_bind, room_origin,
                            state / "owner-context.cbor", state / "executions.jsonl",
                            state / "crashed-once"], "harness")

    def run_json(command):
        completed = subprocess.run([str(v) for v in command], capture_output=True, timeout=15)
        if completed.returncode:
            raise RuntimeError(completed.stderr.decode(errors="replace")[-2000:])
        for line in reversed(completed.stdout.splitlines()):
            try:
                return json.loads(line)
            except (ValueError, UnicodeDecodeError):
                pass
        raise RuntimeError("fixture command returned no result")

    def db_rows(query):
        with sqlite3.connect(state / "room.db") as database:
            return database.execute(query).fetchall()

    try:
        room = room_start("room-initial")
        append = run_json([args.room_bin, "send", room_origin, gateway_url, state])
        (state / "append-result.json").write_text(json.dumps(append, indent=2))
        # The gateway is not running yet. Kill the service with its sole durable
        # wake still pending; recovery cannot rely on an in-memory notification.
        room.kill()
        room.wait(timeout=5)
        dispatches = db_rows("SELECT delivery_id FROM subscription_deliveries")
        assert dispatches == [(append["dispatch_id"],)], dispatches
        wake_payloads = db_rows("SELECT payload FROM seated_wakes")
        assert len(wake_payloads) == 1
        start("gateway", [args.gateway_bin, gateway_bind, harness_origin,
                          state / "unused-settlement-journal"], "gateway")
        harness = harness_start("harness-before-crash")
        # Bad authentication travels through actual gateway dispatch. The room
        # is down, so this also proves rejection before room access/execution.
        forged = urllib.request.Request(gateway_url, data=b"{}", method="POST", headers={
            "webhook-id": "forged", "webhook-timestamp": str(int(time.time())),
            "webhook-signature": "v1," + base64.b64encode(bytes(32)).decode(),
        })
        try:
            urllib.request.urlopen(forged, timeout=10)
            raise AssertionError("forged wake was accepted")
        except urllib.error.HTTPError as failure:
            assert failure.code == 401, failure.code
        assert not (state / "executions.jsonl").exists()

        room_start("room-recovered")
        # The harness example exits only after a successful encrypted append and
        # before its route acknowledgement. This is an actual process crash.
        assert harness.wait(timeout=35) == 99
        assert (state / "crashed-once").exists()
        messages = db_rows("SELECT content FROM messages WHERE room_id='10000000-0000-4000-8000-000000000001'")
        assert len(messages) == 2 and all(row[0].startswith("cow1:") for row in messages)
        assert db_rows("SELECT delivery_id FROM subscription_deliveries") == dispatches
        assert db_rows("SELECT payload FROM seated_wakes") == wake_payloads
        harness_start("harness-recovered")
        deadline = time.monotonic() + 35
        last_error = None
        while time.monotonic() < deadline:
            try:
                result = run_json([args.room_bin, "verify", room_origin, state])
                break
            except RuntimeError as error:
                last_error = error
                time.sleep(0.2)
        else:
            raise RuntimeError(f"reply/acknowledgement recovery timed out: {last_error}")
        executions = [json.loads(line) for line in (state / "executions.jsonl").read_text().splitlines()]
        assert len(executions) >= 2
        assert {item["dispatch_id"] for item in executions} == {append["dispatch_id"]}
        assert {item["message_id"] for item in executions} == {append["message_id"]}
        for path in state.iterdir():
            if path.is_file():
                content = path.read_bytes()
                assert b"fixture private trigger" not in content, path.name
                assert b"fixture actor reply" not in content, path.name
        report = {
            "scope": "local fixture credentials and runtime; no production key-release or billing claim",
            "artifacts": str(state), "append": append, "reply": result,
            "handler_executions": len(executions), "forged_wake_status": 401,
            "room_process_restarted": True, "harness_crashed_after_append_before_ack": True,
            "same_durable_dispatch_after_restarts": True,
            "same_durable_wake_bytes_after_restarts": True,
            "actual_gateway_dispatch_and_harness_handler": True,
            "observed_plaintext_in_captured_state_or_logs": False,
            "provider_specific_apis_used": [], "consensus_calls": 0,
        }
        (state / "report.json").write_text(json.dumps(report, indent=2))
        print(json.dumps(report, indent=2))
    except Exception:
        print(f"Fixture artifacts retained at {state}")
        raise
    finally:
        for process in reversed(processes):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)
        for log in logs:
            log.close()


if __name__ == "__main__":
    main()
