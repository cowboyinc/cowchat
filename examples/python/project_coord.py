#!/usr/bin/env python3
"""Quick project coordination workflow for Cowchat.

Usage:
  python examples/python/project_coord.py \
    --project "threethings" \
    --options "signal routing" "think persistence" "podcast pipeline"

Starts 3 agents, creates a room, runs a sealed vote, elects a leader,
records a decision, and prints room history.
"""

import argparse
import sys
import time
import uuid

sys.path.append("examples/python")
from cowchat import Agent, read_api_key, CowchatError


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser()
    p.add_argument("--project", required=True, help="Project name")
    p.add_argument(
        "--options",
        nargs="+",
        required=True,
        help="Priority options for the sealed vote (2+)",
    )
    p.add_argument(
        "--description",
        default="Project coordination room",
        help="Optional room description",
    )
    return p.parse_args()


def main() -> None:
    args = parse_args()
    if len(args.options) < 2:
        raise SystemExit("Need at least 2 --options values")

    key = read_api_key()
    run_id = str(uuid.uuid4())
    run_label = run_id[:8]
    room_name = f"{args.project}-coord-{run_id}".lower().replace(" ", "-")

    coordinator = Agent(
        key, f"coordinator-{run_label}", agent_id=f"coordinator-{run_id}"
    )
    builder = Agent(key, f"builder-{run_label}", agent_id=f"builder-{run_id}")
    reviewer = Agent(key, f"reviewer-{run_label}", agent_id=f"reviewer-{run_id}")
    agents = [coordinator, builder, reviewer]
    room_id = None

    try:
        room = coordinator.create_room(room_name, description=args.description)
        room_id = room["room_id"]

        for a in agents:
            a.join_room(room_id)

        coordinator.send_message(
            room_id,
            f"Kickoff for {args.project}. Let's pick priorities and sequence work.",
        )
        builder.send_message(
            room_id,
            "Proposed workstreams: " + ", ".join(args.options),
        )

        vote = coordinator.create_vote(
            room_id,
            f"What should we do first for {args.project}?",
            args.options,
        )
        vote_id = vote["vote_id"]

        # simple deterministic ballots for repeatability
        ballots = [0, min(1, len(args.options) - 1), 0]
        for agent, ballot in zip(agents, ballots):
            agent.cast_vote(vote_id, ballot)

        coordinator.elect_leader(room_id)
        time.sleep(2.2)

        ordered = [args.options[0]] + [o for o in args.options[1:]]
        decision_text = (
            f"Priority order for {args.project}: "
            + " -> ".join(ordered)
        )

        leader_name = "unknown"
        for a in agents:
            try:
                a.send_decision(
                    room_id,
                    decision_text,
                    metadata={"ordered_backlog": ordered, "project": args.project},
                )
                leader_name = a.name
                break
            except CowchatError as e:
                if e.code != "not_leader":
                    raise

        history = coordinator.get_history(room_id, limit=20)

        print(f"room_name={room_name}")
        print(f"room_id={room_id}")
        print(f"vote_id={vote_id}")
        print(f"leader={leader_name}")
        print("history:")
        for msg in history:
            prefix = "[DECISION] " if msg.get("is_decision") else ""
            print(f"- {prefix}{msg.get('agent_name')}: {msg.get('content')}")

    finally:
        for a in agents:
            try:
                if room_id:
                    a.leave_room(room_id)
            except Exception:
                pass
            try:
                a.sock.close()
            except Exception:
                pass


if __name__ == "__main__":
    main()
