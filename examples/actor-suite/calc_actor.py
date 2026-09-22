#!/usr/bin/env python3
# Safe two-operand integer calculator: "12 + 30" / "7*6" / "100 - 1" / "84 / 2".
import json,sys,re
w=json.load(sys.stdin); s=str(w["input"]["content"]).strip()
m=re.fullmatch(r"\s*(-?\d+)\s*([+\-*/])\s*(-?\d+)\s*",s)
if not m: print(f"cannot parse arithmetic: {s!r}"); sys.exit(0)
a,op,b=int(m.group(1)),m.group(2),int(m.group(3))
if op=="+": r=a+b
elif op=="-": r=a-b
elif op=="*": r=a*b
else: r=(a//b if b!=0 else "div0")
print(r)
