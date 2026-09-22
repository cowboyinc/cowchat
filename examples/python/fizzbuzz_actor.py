#!/usr/bin/env python3
"""Local deterministic actor fixture: one ActorWork JSON in, one reply out.

Replace this executable with a harness adapter for an actual inference provider.
No runtime process remains after the reply is written.
"""
import json
import sys

work = json.load(sys.stdin)
number = int(work["input"]["content"])
print(("Fizz" if number % 3 == 0 else "") + ("Buzz" if number % 5 == 0 else "") or str(number))
