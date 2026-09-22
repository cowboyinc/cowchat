#!/usr/bin/env python3
# Stateful accumulator: running total persisted to a file (state belongs to the
# adapter, per actor-host docs — the host itself is stateless per input).
import json,sys,os,tempfile,fcntl
w=json.load(sys.stdin)
try: n=int(str(w["input"]["content"]).strip())
except ValueError: print("send an integer to accumulate"); sys.exit(0)
path=os.path.join(os.environ.get("SUM_STATE_DIR",tempfile.gettempdir()),"actor_sum_total.txt")
with open(path,"a+") as f:
    fcntl.flock(f,fcntl.LOCK_EX); f.seek(0); cur=f.read().strip()
    total=(int(cur) if cur else 0)+n
    f.seek(0); f.truncate(); f.write(str(total))
print(total)
