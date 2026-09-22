#!/usr/bin/env python3
# Deterministic magic-8-ball: hash the question so tests are reproducible.
import json,sys,hashlib
ANS=["It is certain","Reply hazy, try again","Don't count on it","Yes definitely","My sources say no","Signs point to yes","Outlook not so good","Without a doubt"]
w=json.load(sys.stdin); q=str(w["input"]["content"])
h=int(hashlib.sha256(q.encode()).hexdigest(),16)
print(ANS[h % len(ANS)])
