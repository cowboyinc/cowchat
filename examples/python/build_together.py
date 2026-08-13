#!/usr/bin/env python3
"""Build Together — 3 Agents Coordinate to Build Tic-Tac-Toe

The capstone Cowchat example: a coordinator, a server-dev, and a
client-dev agent work together to "build" a tic-tac-toe game.

Demonstrates ALL coordination features: rooms, sub-rooms, messaging,
sealed-ballot voting, leader election, and decisions.

Usage:
    cargo run -p cowchat-server -- serve
    python examples/python/build_together.py
"""

import time
import uuid
from cowchat import Agent, read_api_key


def main():
    key = read_api_key()
    run_id = uuid.uuid4().hex[:6]

    # ========================================================
    print("\n=== PHASE 1: SETUP & PLANNING ===\n")
    # ========================================================

    coordinator = Agent(key, f"coordinator-{run_id}")
    server_dev = Agent(key, f"server-dev-{run_id}")
    client_dev = Agent(key, f"client-dev-{run_id}")
    print("  Connected 3 agents:")
    print(f"    coordinator: {coordinator.agent_id}")
    print(f"    server-dev:  {server_dev.agent_id}")
    print(f"    client-dev:  {client_dev.agent_id}")

    # Coordinator creates the project room
    room_name = f"tictactoe-{run_id}"
    project = coordinator.create_room(room_name, description="Tic-tac-toe game project")
    project_id = project["room_id"]
    print(f"\n  Created project room: {room_name} ({project_id})")

    # Everyone joins
    coordinator.join_room(project_id)
    server_dev.join_room(project_id)
    client_dev.join_room(project_id)
    print("  All agents joined\n")

    # Coordinator kicks things off
    coordinator.send_message(project_id,
        "Let's build a tic-tac-toe game. We need a TCP server and a client.")
    print("  coordinator: Let's build a tic-tac-toe game.")

    # Create sub-rooms for focused work
    server_room = coordinator.create_room(
        f"ttt-server-{run_id}", parent_id=project_id)
    client_room = coordinator.create_room(
        f"ttt-client-{run_id}", parent_id=project_id)
    print(f"  Created sub-rooms: ttt-server-{run_id}, ttt-client-{run_id}")

    server_dev.join_room(server_room["room_id"])
    client_dev.join_room(client_room["room_id"])
    print(f"  server-dev -> ttt-server, client-dev -> ttt-client")

    # ========================================================
    print("\n=== PHASE 2: VOTE ON PROTOCOL ===\n")
    # ========================================================

    vote = coordinator.create_vote(
        project_id,
        "Wire protocol format?",
        ["JSON", "plain text", "binary"],
        description="How should the server and client talk to each other?",
    )
    print('  Vote: "Wire protocol format?"')
    print("  Options: JSON | plain text | binary")
    print(f"  Eligible: {vote['eligible_voters']}\n")

    # Cast sealed ballots
    print("  Casting sealed ballots...")
    coordinator.cast_vote(vote["vote_id"], 0)  # JSON
    print("    coordinator -> (sealed)")
    server_dev.cast_vote(vote["vote_id"], 0)   # JSON
    print("    server-dev  -> (sealed)")
    client_dev.cast_vote(vote["vote_id"], 1)   # plain text
    print("    client-dev  -> (sealed)")

    # Wait for results
    result = coordinator.wait_for_event("vote_result")["payload"]
    print("\n  Results revealed:")
    for entry in result.get("tally", []):
        option = entry["option_text"]
        count = entry["count"]
        bar = "#" * (count * 4)
        print(f"    {option:<12} {bar} ({count})")

    ballots = result.get("ballots", [])
    if ballots:
        print()
        options = ["JSON", "plain text", "binary"]
        for ballot in ballots:
            name = ballot["agent_name"]
            choice = options[ballot["option_index"]]
            print(f"    {name:<14} voted {choice}")

    coordinator.send_message(project_id, "Vote result: JSON wins! We'll use JSON.")

    # ========================================================
    print("\n=== PHASE 3: ELECT TECH LEAD ===\n")
    # ========================================================

    coordinator.elect_leader(project_id)
    print("  Election started (2s opt-out window)...")

    # Wait for election result
    elected = server_dev.wait_for_event("leader_elected", timeout=5.0)["payload"]
    leader_id = elected["leader_id"]
    leader_name = elected["leader_name"]
    print(f"  Leader elected: {leader_name}")

    # Leader issues the protocol decision
    protocol_spec = ('JSON protocol decided: {"action":"move","pos":0-8} for moves, '
                     '{"state":"board","cells":["X","O"," ",...]} for state updates')

    agents = {coordinator.agent_id: coordinator,
              server_dev.agent_id: server_dev,
              client_dev.agent_id: client_dev}
    leader = agents[leader_id]
    leader.send_decision(project_id, protocol_spec)

    decision = coordinator.wait_for_event("decision_made")["payload"]
    content = decision.get("content", "?")
    print(f'\n  Decision by {decision.get("leader_name", "?")}: "{content[:60]}"')

    # ========================================================
    print("\n=== PHASE 4: BUILD ===\n")
    # ========================================================

    server_dev.send_message(server_room["room_id"],
        "Building game server... TCP listener, move validation, win detection")
    print("  [ttt-server] server-dev: Building game server...")

    client_dev.send_message(client_room["room_id"],
        "Building game client... board renderer, input parser, TCP connector")
    print("  [ttt-client] client-dev: Building game client...")

    # Simulate build time
    time.sleep(0.5)
    print("  Building", end="", flush=True)
    for _ in range(3):
        time.sleep(0.3)
        print(".", end="", flush=True)
    print()

    server_dev.send_message(server_room["room_id"],
        "Server done! Listening on port 3000. Supports 2-player matches.")
    print("\n  [ttt-server] server-dev: Server done! Listening on port 3000")

    client_dev.send_message(client_room["room_id"],
        "Client done! Connects to server:3000, renders board in terminal.")
    print("  [ttt-client] client-dev: Client done! Connects to server:3000")

    # Report back to main room
    server_dev.send_message(project_id, "Server component ready!")
    client_dev.send_message(project_id, "Client component ready!")
    print("\n  Both devs reported ready in main room")

    # ========================================================
    print("\n=== PHASE 5: INTEGRATION ===\n")
    # ========================================================

    coordinator.send_message(project_id,
        "All components ready. Tic-tac-toe is shipped! Great work team.")
    print("  coordinator: All components ready. Ship it!")

    # Fetch and print the full project history
    history = coordinator.get_history(project_id, limit=50)

    print(f"\n=== PROJECT HISTORY ({len(history)} messages) ===\n")
    for msg in history:
        name = msg.get("agent_name", "?")
        content = msg.get("content", "")
        metadata = msg.get("metadata") or {}
        is_decision = metadata.get("type") == "decision"
        if is_decision:
            print(f"  [DECISION] {name}: {content}")
        else:
            print(f"  {name}: {content}")

    # Clean up
    server_dev.leave_room(server_room["room_id"])
    client_dev.leave_room(client_room["room_id"])
    print("\n  Sub-rooms left")

    print("\nDone!")


if __name__ == "__main__":
    main()
