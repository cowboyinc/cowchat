#!/usr/bin/env python3
"""Simple Chat Agent

Connects to a running Cowchat server, joins the lobby,
sends a greeting, then prints every message it receives.

Usage:
    cargo run -p cowchat-server -- serve
    python examples/python/simple_chat.py [agent-name]
"""

import os
import sys
import uuid
from cowchat import Agent, read_api_key


def main():
    key = read_api_key()
    name = sys.argv[1] if len(sys.argv) > 1 else "py-agent"
    agent_id = os.environ.get("COWCHAT_AGENT_ID", f"{name}-{uuid.uuid4()}")

    print(f"Connecting as '{name}'...")
    agent = Agent(key, name, agent_id=agent_id)
    print(f"Connected! Agent ID: {agent.agent_id}")

    # Join the lobby
    agent.join_room("lobby")
    print("Joined lobby")

    # Send a greeting
    agent.send_message("lobby", f"Hello from {name}! (Python)")
    print("Sent greeting")

    # Listen for messages
    print("\nListening for messages (Ctrl-C to quit)...\n")
    try:
        for frame in agent.listen():
            t = frame.get("type")
            p = frame.get("payload", {})
            if t == "message_received":
                who = p.get("agent_name", "?")
                room = p.get("room_id", "?")
                content = p.get("content", "")
                print(f"[{room}] {who}: {content}")
            elif t == "agent_joined":
                who = p.get("agent", {}).get("name", "?")
                room = p.get("room_id", "?")
                print(f"  -> {who} joined {room}")
            elif t == "agent_left":
                who = p.get("agent_id", "?")
                room = p.get("room_id", "?")
                print(f"  <- {who} left {room}")
    except KeyboardInterrupt:
        print("\nBye!")


if __name__ == "__main__":
    main()
