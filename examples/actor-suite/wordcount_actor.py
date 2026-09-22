#!/usr/bin/env python3
import json,sys
w=json.load(sys.stdin); print(len(str(w["input"]["content"]).split()))
