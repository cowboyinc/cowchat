#!/usr/bin/env python3
# Tiny sentiment/keyword classifier.
import json,sys
POS={"love","great","good","awesome","happy","yes","win"}
NEG={"hate","bad","awful","terrible","sad","no","fail","broken"}
w=json.load(sys.stdin); toks=set(str(w["input"]["content"]).lower().split())
p,n=len(toks&POS),len(toks&NEG)
print("positive" if p>n else "negative" if n>p else "neutral")
