#!/usr/bin/env python3
"""Leader Election & Decision

Spawns 3 agents, runs a leader election (one agent declines),
then the elected leader issues a binding decision.

Elections have a 2-second opt-out window. After that, the server
picks randomly from remaining candidates.

Usage:
    cargo run -p cowchat-server -- serve
    python examples/python/leader_election.py
"""

import uuid
from cowchat import Agent, CowchatError, read_api_key


def main():
    key = read_api_key()
    run_id = str(uuid.uuid4())
    run_label = run_id[:8]

    # Connect 3 agents
    print("Connecting agents...")
    lead = Agent(key, f"lead-{run_label}", agent_id=f"lead-{run_id}")
    dev1 = Agent(key, f"dev-1-{run_label}", agent_id=f"dev-1-{run_id}")
    dev2 = Agent(key, f"dev-2-{run_label}", agent_id=f"dev-2-{run_id}")
    print(f"  lead:  {lead.agent_id}")
    print(f"  dev-1: {dev1.agent_id}")
    print(f"  dev-2: {dev2.agent_id}")

    # Create a room and join
    room = lead.create_room(
        f"sprint-{run_id}", description="Sprint planning session"
    )
    room_id = room["room_id"]
    print(f"\nCreated room: {room['name']} ({room_id})")

    lead.join_room(room_id)
    dev1.join_room(room_id)
    dev2.join_room(room_id)
    print("All agents joined\n")

    # Start the election
    print("Starting leader election...")
    lead.elect_leader(room_id)

    # Wait for ElectionStarted
    started = lead.wait_for_event("election_started")["payload"]
    candidates = started.get("candidates", [])
    print(f"  Candidates: {', '.join(candidates)}")
    print(f"  Opt-out window: {started.get('opt_out_seconds', 2)}s")

    # dev-2 declines
    dev2.decline_election(room_id)
    print("\n  dev-2 declined candidacy")
    print("  Waiting for election to complete...\n")

    # Wait for LeaderElected
    elected = dev1.wait_for_event("leader_elected", timeout=5.0)["payload"]
    leader_id = elected["leader_id"]
    leader_name = elected["leader_name"]
    print("=== LEADER ELECTED ===")
    print(f"  {leader_name} ({leader_id})\n")

    # The elected leader issues a decision
    decision_text = "We ship the auth service this sprint using Rust"
    print("Leader issuing decision...")

    agents = {lead.agent_id: lead, dev1.agent_id: dev1, dev2.agent_id: dev2}
    leader = agents[leader_id]
    leader.send_decision(room_id, decision_text)

    # Wait for DecisionMade event on a non-leader
    non_leader = lead if leader_id != lead.agent_id else dev1
    decision = non_leader.wait_for_event("decision_made")["payload"]
    print("\n=== DECISION MADE ===")
    print(f"  By:      {decision.get('leader_name', '?')}")
    print(f'  Content: "{decision.get("content", "?")}"')

    # Show that a non-leader gets rejected
    print("\nVerifying non-leader cannot issue decisions...")
    try:
        non_leader.send_decision(room_id, "rogue decision")
        print("  ERROR: non-leader decision was accepted!")
    except CowchatError as e:
        print(f"  Correctly rejected: {e}")

    print("\nDone!")


if __name__ == "__main__":
    main()
