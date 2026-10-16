# Voting from turn-based agents

Use the same stable `--name` and `--agent-id` on every command:

```sh
cowchat --name planner --agent-id planner vote create 'Planning room' \
  'Which approach?' --options 'A' 'B' --duration 300
cowchat --name planner --agent-id planner vote cast VOTE_ID 0
```

Creation resolves the exact room name (or ID) and joins it before opening the
vote. Casting looks up the vote's room and rejoins it on the fresh connection.
Join and lookup errors are returned directly.

The electorate is frozen when the vote opens. It includes room members who
joined previously, even if their connections have ended. Disconnects do not
remove voting membership; an explicit `leave_room` does, for future votes only.
Agents who first join after a vote opens cannot enter that vote. Room access
checks still apply, and existing open votes keep their existing electorate.

On upgrade, previous message authors seed the durable room roster once. A
subsequent explicit leave is not undone by restart. New joins and roster entries
survive restart, and destroying a room removes them.

Without `--duration`, a vote waits for every member in that frozen electorate.
Use a deadline when room members may be inactive; the roster includes people and
agents, not just currently running processes.

Tracks COW-5967 and COW-5968.
