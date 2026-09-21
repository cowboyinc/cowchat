#!/usr/bin/env python3
"""Deterministic multi-actor coordination fixtures, one role per seat.

Ports of cowboy repo examples onto Cowchat rooms:
  ring       gallery/advanced-messaging-ring  — token passed actor-to-actor
  validator  core/04-multi-actor-workflow     — validate, hand to settlement
  ledger     core/04-multi-actor-workflow     — record the settlement
  signer     core/13-multisig-treasury        — approve a proposal
  treasurer  core/13-multisig-treasury        — execute at 2 approvals
  casino     gallery/casino-rounds            — open round, take bets, settle
  feed       gallery/casino-rounds            — supply the settlement price
  bettor     gallery/casino-rounds            — bet on any open round

Protocol: one ActorWork JSON on stdin. Empty stdout skips the work; a JSON
object {"reply": ..., "mentions": [...]} routes follow-on work to other
actors; other output is a plain reply. Stateful roles keep a line-per-event
file at $COORD_STATE (the durable private state directory pattern).
"""
import json
import os
import sys


def reply(text, *mentions):
    print(json.dumps({"reply": text, "mentions": list(mentions)}))


def state_append(line):
    with open(os.environ["COORD_STATE"], "a") as f:
        f.write(line + "\n")


def state_lines(prefix):
    try:
        with open(os.environ["COORD_STATE"]) as f:
            return [l.strip() for l in f if l.startswith(prefix)]
    except FileNotFoundError:
        return []


role = sys.argv[1]
work = json.load(sys.stdin)
content = work["input"]["content"].strip()

if role == "ring":
    me, successor = sys.argv[2], sys.argv[3]
    token = json.loads(content)
    token["path"].append(me)
    if token["hops"] == 0:
        print("ring-complete path=" + ">".join(token["path"]))
    else:
        token["hops"] -= 1
        reply(json.dumps(token), successor)

elif role == "validator":
    answer = content.removeprefix("submit:")
    verdict = "valid" if answer == "42" else "invalid"
    reply(f"{verdict}:{answer}", "ledger")

elif role == "ledger":
    state_append("record " + content)
    print(f"recorded #{len(state_lines('record '))}: {content}")

elif role == "signer":
    proposal = content.removeprefix("proposal:")
    reply(f"approve:{proposal}:{sys.argv[2]}", "treasurer")

elif role == "treasurer":
    if not content.startswith("approve:"):
        sys.exit(0)  # empty stdout: skip non-approval chatter
    proposal = content.split(":")[1]
    state_append(f"approval {proposal}")
    if len(state_lines(f"approval {proposal}")) == 2:
        print(f"executed:{proposal}")
    # first approval: quorum not reached, skip (empty stdout)

elif role == "casino":
    if content.startswith("open:"):
        round_id = content.removeprefix("open:")
        state_append(f"open {round_id}")
        reply(f"round-open:{round_id}", "feed")
    elif content.startswith("bet:"):
        state_append("bet " + content)
    elif content.startswith("price:"):
        _, round_id, price = content.split(":")
        bets = len(state_lines("bet "))
        print(f"settled:{round_id} price={price} bets={bets}")
    # anything else (or a recorded bet): skip, no reply

elif role == "feed":
    if content.startswith("round-open:"):
        round_id = content.removeprefix("round-open:")
        reply(f"price:{round_id}:117", "casino")

elif role == "bettor":
    if content.startswith("round-open:"):
        round_id = content.removeprefix("round-open:")
        reply(f"bet:{round_id}:{sys.argv[2]}", "casino")
    # everything else in the room: skip

else:
    sys.exit(f"unknown role {role}")
