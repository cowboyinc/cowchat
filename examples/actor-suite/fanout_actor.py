#!/usr/bin/env python3
# Fan-out coordinator: one actor triggers MANY. Forwards its input to every
# worker agent-id in argv via a single addressed reply. argv: <worker...>.
import json,sys
workers=sys.argv[1:]
w=json.load(sys.stdin); content=str(w["input"]["content"])
print(json.dumps({"reply":content,"mentions":workers}))
