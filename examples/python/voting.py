#!/usr/bin/env python3
"""Sealed-Ballot Voting

Spawns 3 agents in-process, creates a sealed vote, has each
cast a ballot, then prints the revealed results.

Votes are sealed: nobody sees anyone's choice until all ballots
are in (or a deadline expires). This prevents anchoring bias.

Usage:
    cargo run -p cowchat-server -- serve
    python examples/python/voting.py
"""

import uuid
from cowchat import Agent, read_api_key


def main():
    key = read_api_key()
    run_id = uuid.uuid4().hex[:6]

    # Connect 3 agents
    print("Connecting agents...")
    alice = Agent(key, f"alice-{run_id}")
    bob = Agent(key, f"bob-{run_id}")
    charlie = Agent(key, f"charlie-{run_id}")
    print(f"  alice:   {alice.agent_id}")
    print(f"  bob:     {bob.agent_id}")
    print(f"  charlie: {charlie.agent_id}")

    # Create a room and join
    room = alice.create_room(f"lang-vote-{run_id}", description="Vote on language choice")
    room_id = room["room_id"]
    print(f"\nCreated room: {room['name']} ({room_id})")

    alice.join_room(room_id)
    bob.join_room(room_id)
    charlie.join_room(room_id)
    print("All agents joined\n")

    # Alice creates a sealed-ballot vote
    vote = alice.create_vote(
        room_id,
        "Which language for the new service?",
        ["Rust", "Go", "Python"],
        description="Pick one -- ballots are sealed until everyone votes",
    )
    print(f'Vote created: "{vote["title"]}"')
    print("  Options: Rust | Go | Python")
    print(f"  Eligible voters: {vote['eligible_voters']}\n")

    # Each agent casts a sealed ballot
    print("Casting sealed ballots...")
    alice.cast_vote(vote["vote_id"], 0)   # Rust
    print("  alice   voted (sealed)")
    bob.cast_vote(vote["vote_id"], 0)     # Rust
    print("  bob     voted (sealed)")
    charlie.cast_vote(vote["vote_id"], 1) # Go
    print("  charlie voted (sealed)")

    # Wait for VoteResult event
    print("\nWaiting for results...\n")
    result = alice.wait_for_event("vote_result")["payload"]

    # Print the results
    print('=== VOTE RESULTS ===')
    print(f'"{result["title"]}"')
    print()

    for entry in result.get("tally", []):
        option = entry["option_text"]
        count = entry["count"]
        bar = "#" * (count * 5)
        print(f"  {option:<8} {bar} ({count})")

    print()
    print("Individual ballots (now revealed):")
    options = ["Rust", "Go", "Python"]
    for ballot in result.get("ballots", []):
        name = ballot["agent_name"]
        choice = options[ballot["option_index"]]
        print(f"  {name:<10} -> {choice}")

    print(f"\nTotal: {result['total_votes']}/{result['eligible_voters']}")


if __name__ == "__main__":
    main()
