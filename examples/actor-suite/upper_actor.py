#!/usr/bin/env python3
import json,sys
w=json.load(sys.stdin); print(str(w["input"]["content"]).upper())
