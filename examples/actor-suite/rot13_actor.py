#!/usr/bin/env python3
import json,sys,codecs
w=json.load(sys.stdin); print(codecs.encode(str(w["input"]["content"]),"rot_13"))
