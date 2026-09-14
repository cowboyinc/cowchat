#!/usr/bin/env python3
"""Run the local actor+builder Cowchat durable paid-recovery proof.

The suite composes the real Cowchat HTTP/SQLite service, generic gateway
router/runner proxy, verified CBSS fixture publication and HTTP release service,
transient Harness room runtime, and Cattle Guard PostgreSQL journal/accounting.
All keys, funding, model output, and discovery state are explicit test fixtures.
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

PRIVATE_CANARIES = (
    b"fixture private trigger for actor",
    b"fixture private trigger for builder",
    b"fixture actor reply",
    b"fixture builder reply",
    b"changed candidate",
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("room", "gateway", "harness"):
        parser.add_argument(f"--{name}-bin", required=True, type=Path)
    parser.add_argument(
        "--database-url",
        required=True,
        help="Disposable loopback PostgreSQL /room_runs_test URL (non-5432 port)",
    )
    args = parser.parse_args()
    state = Path(tempfile.mkdtemp(prefix="cowchat-m-echo-"))
    room_state = state / "room"
    harness_state = state / "harness"
    processes = []
    logs = []

    def port():
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            return listener.getsockname()[1]

    room_bind, gateway_bind, harness_bind = [f"127.0.0.1:{port()}" for _ in range(3)]
    room_origin = f"http://{room_bind}"
    gateway_origin = f"http://{gateway_bind}"
    harness_origin = f"http://{harness_bind}"

    def start(label, command, field):
        stderr = (state / f"{label}.stderr").open("wb")
        logs.append(stderr)
        process = subprocess.Popen(
            [str(value) for value in command],
            stdout=subprocess.PIPE,
            stderr=stderr,
        )
        processes.append(process)
        deadline = time.monotonic() + 20
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
                    return process, item
        raise RuntimeError(f"{label} readiness timeout")

    def harness_start(label):
        return start(
            label,
            [
                args.harness_bin,
                "serve",
                harness_bind,
                room_origin,
                args.database_url,
                harness_state,
                state / "crashed-once",
            ],
            "harness",
        )

    def room_start(label, manifest):
        return start(
            label,
            [args.room_bin, "serve", room_bind, room_state, manifest],
            "room_origin",
        )

    def run_json(command, timeout=20):
        completed = subprocess.run(
            [str(value) for value in command],
            capture_output=True,
            timeout=timeout,
        )
        if completed.returncode:
            raise RuntimeError(completed.stderr.decode(errors="replace")[-2000:])
        for line in reversed(completed.stdout.splitlines()):
            try:
                return json.loads(line)
            except (ValueError, UnicodeDecodeError):
                pass
        raise RuntimeError("fixture command returned no JSON result")

    def room_rows(query, parameters=()):
        with sqlite3.connect(room_state / "room.db") as database:
            return database.execute(query, parameters).fetchall()

    def harness_status():
        with urllib.request.urlopen(
            f"{harness_origin}/_fixture/status", timeout=5
        ) as response:
            return json.load(response)

    def roles(manifest):
        return {seat["role"]: seat for seat in manifest["seats"]}

    try:
        harness, harness_ready = harness_start("harness-before-crash")
        manifest_path = Path(harness_ready["manifest"])
        manifest = json.loads(manifest_path.read_text())
        seats = roles(manifest)
        assert set(seats) == {"actor", "builder"}, seats

        room, room_ready = room_start("room-initial", manifest_path)
        append = run_json(
            [
                args.room_bin,
                "send",
                room_origin,
                gateway_origin,
                room_state,
                manifest_path,
            ]
        )
        (state / "append-result.json").write_text(json.dumps(append, indent=2))

        # The generic gateway is absent. Stop Cowchat with both exact seated wake
        # payloads durable; delivery cannot depend on an in-memory notification.
        room.kill()
        room.wait(timeout=5)
        dispatches = room_rows(
            "SELECT subscription_id,delivery_id,message_id FROM subscription_deliveries ORDER BY message_id"
        )
        wake_payloads = room_rows(
            "SELECT delivery_id,payload FROM seated_wakes ORDER BY delivery_id"
        )
        assert len(dispatches) == 2, dispatches
        assert len(wake_payloads) == 2, wake_payloads
        expected_dispatches = {
            item["message_id"]: item["dispatch_id"] for item in append["messages"]
        }
        assert {
            message: dispatch for _, dispatch, message in dispatches
        } == expected_dispatches

        gateway, gateway_ready = start(
            "gateway",
            [
                args.gateway_bin,
                gateway_bind,
                harness_origin,
                manifest_path,
                state / "unused-settlement-journal",
            ],
            "gateway",
        )
        assert set(gateway_ready["actors"]) == {
            seats["actor"]["gateway_actor"],
            seats["builder"]["gateway_actor"],
        }

        # Bad Standard Webhooks authentication traverses the actual gateway and
        # is refused before durable admission, room access, or paid execution.
        forged_url = (
            f"{gateway_origin}/{seats['actor']['gateway_actor']}/wake"
        )
        forged = urllib.request.Request(
            forged_url,
            data=b"{}",
            method="POST",
            headers={
                "webhook-id": "forged",
                "webhook-timestamp": str(int(time.time())),
                "webhook-signature": "v1,"
                + base64.b64encode(bytes(32)).decode(),
            },
        )
        try:
            urllib.request.urlopen(forged, timeout=10)
            raise AssertionError("forged wake was accepted")
        except urllib.error.HTTPError as failure:
            assert failure.code == 401, failure.code
        before = harness_status()
        assert before["runs"] == [], before
        assert before["model_calls"] == {"actor": 0, "builder": 0}, before
        assert before["privacy"] == {
            "postgres_plaintext_matches": 0,
            "room_content_sink_rows": 0,
        }

        room, _ = room_start("room-recovered", manifest_path)

        # The Harness fixture exits after one authenticated encrypted reply and
        # final accounting succeed, but before Cattle Guard marks that run done.
        assert harness.wait(timeout=25) == 99
        assert (state / "crashed-once").exists()
        events_after_crash = [
            json.loads(line)
            for line in (harness_state / "executions.jsonl").read_text().splitlines()
        ]
        assert len(events_after_crash) == 1, events_after_crash
        messages_after_crash = room_rows(
            "SELECT message_id,content FROM messages WHERE room_id=? ORDER BY seq",
            (manifest["room"],),
        )
        assert len(messages_after_crash) == 3, messages_after_crash
        assert all(content.startswith("cow1:") for _, content in messages_after_crash)

        harness, _ = harness_start("harness-recovered")
        deadline = time.monotonic() + 25
        last_error = None
        result = None
        status = None
        while time.monotonic() < deadline:
            try:
                status = harness_status()
                result = run_json(
                    [
                        args.room_bin,
                        "verify",
                        room_origin,
                        room_state,
                        manifest_path,
                    ],
                    timeout=10,
                )
                if (
                    len(status["runs"]) == 2
                    and all(run["status"] == "completed" for run in status["runs"])
                    and status["model_calls"] == {"actor": 1, "builder": 1}
                ):
                    break
            except (
                AssertionError,
                KeyError,
                RuntimeError,
                urllib.error.URLError,
            ) as error:
                last_error = error
            time.sleep(0.2)
        else:
            raise RuntimeError(
                f"paid reply/reconciliation recovery timed out: {last_error}; status={status}"
            )

        assert result["pending_wakes"] == 0, result
        assert status["accounting"] == {
            "actor": {"credit": 1000, "held": 0, "charged": 6},
            "builder": {"credit": 1000, "held": 0, "charged": 6},
        }, status
        assert status["privacy"] == {
            "postgres_plaintext_matches": 0,
            "room_content_sink_rows": 0,
        }, status
        assert status["consensus_calls"] == 0
        assert not status["cbss"]["finished"]
        assert not status["cbss"]["writers_finished"]
        assert sum(status["cbss"]["issuance"]) > 0
        assert sorted(run["lease_epoch"] for run in status["runs"]) == [1, 2]
        assert {
            run["trigger"]: run["dispatch"] for run in status["runs"]
        } == expected_dispatches
        assert {run["seat"] for run in status["runs"]} == {
            seats["actor"]["seat"],
            seats["builder"]["seat"],
        }

        events = [
            json.loads(line)
            for line in (harness_state / "executions.jsonl").read_text().splitlines()
        ]
        assert len(events) == 2, events
        assert sorted(event["role"] for event in events) == ["actor", "builder"]

        for path in state.rglob("*"):
            if path.is_file():
                content = path.read_bytes()
                for canary in PRIVATE_CANARIES:
                    assert canary not in content, (path, canary)

        report = {
            "scope": (
                "local fixture keys/funding/model/discovery; CBSS runs inside the "
                "supervised Harness fixture process"
            ),
            "artifacts": str(state),
            "room": room_ready,
            "gateway": gateway_ready,
            "append": append,
            "reply": result,
            "cattle_guard": {
                "runs": status["runs"],
                "accounting": status["accounting"],
                "one_recovered_lease": True,
                "one_model_call_per_seat": True,
            },
            "cbss": status["cbss"],
            "forged_wake_status": 401,
            "room_process_restarted_with_two_durable_wakes": True,
            "harness_crashed_after_reply_and_accounting_before_run_completion": True,
            "stable_dispatch_recovered_from_postgres": True,
            "actual_generic_gateway_dispatch": True,
            "actual_cowchat_sqlite_transport": True,
            "observed_plaintext_in_captured_files_or_postgres_sinks": False,
            "provider_specific_apis_used": [],
            "consensus_calls": 0,
            "not_claimed": [
                "real CBQS transport",
                "CBFS archive",
                "provider delivery",
                "separate-process or independently operated CBSS",
                "production key custody or funding",
                "deployed network readiness",
            ],
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
