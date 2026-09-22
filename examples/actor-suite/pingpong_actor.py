#!/usr/bin/env python3
# Actor-to-actor bounce. argv: <self-label> <partner-agent-id>.
# Reads the trailing integer in the content; if >0, bounces to the partner with
# N-1 via an addressed reply {reply, mentions}; at 0, replies plain text to STOP.
import json,sys,re
label,partner=sys.argv[1],sys.argv[2]
w=json.load(sys.stdin); content=str(w["input"]["content"])
m=re.search(r"(-?\d+)\s*$",content)
n=int(m.group(1)) if m else 0
if n<=0:
    print(f"done at {label}")            # plain reply, no mention -> chain stops
else:
    print(json.dumps({"reply":f"{label} {n-1}","mentions":[partner]}))
