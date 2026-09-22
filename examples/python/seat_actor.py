#!/usr/bin/env python3
"""One FizzBuzz seat: role from argv, one ActorWork JSON in, one reply out.

Roles: counter replies the next integer; fizz replies "Fizz" when the number
is divisible by 3, buzz replies "Buzz" when divisible by 5, otherwise "pass".
Deterministic local fixture; swap for an inference adapter later.
"""
import json
import sys

role = sys.argv[1]
number = int(json.load(sys.stdin)["input"]["content"])
if role == "counter":
    reply = str(number + 1)
elif role == "fizz":
    reply = "Fizz" if number % 3 == 0 else "pass"
elif role == "buzz":
    reply = "Buzz" if number % 5 == 0 else "pass"
else:
    sys.exit(f"unknown role {role}")
sys.stdout.write(reply)
