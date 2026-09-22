#!/usr/bin/env python3
# Multi-hop relay. argv: <self-label> <next-agent-id-or-END>.
# Appends its label to the comma path; forwards to the next hop, or (END) emits
# the final path as a plain reply that terminates the chain.
import json,sys
label,nxt=sys.argv[1],sys.argv[2]
w=json.load(sys.stdin); content=str(w["input"]["content"]).strip()
path=(content+","+label) if content and content!="start" else label
if nxt=="END":
    print(path)                          # plain reply -> stops
else:
    print(json.dumps({"reply":path,"mentions":[nxt]}))
