#!/usr/bin/env bash
# Actor-to-actor messaging: actors mention each OTHER (not just reply to Chad).
# ping-pong bounces a decrementing counter between two actors until it stops;
# relay forwards a growing path A->B->C. Fully local.
set -uo pipefail
cd "$(dirname "$0")/../.."
BIN=target/debug
[ -x $BIN/cowchat-server ] && [ -x $BIN/cowchat ] || { echo "build first"; exit 2; }
DIR=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$DIR"' EXIT
export COWCHAT_ROOM_KEY=a2a-secret
$BIN/cowchat-server serve --socket "$DIR/s.sock" --tcp 127.0.0.1:19529 \
  --db "$DIR/s.db" --key-file "$DIR/auth.key" --allow-private-webhooks >"$DIR/server.log" 2>&1 &
sleep 1.5
KEY=$(cat "$DIR/auth.key"); CC="$BIN/cowchat --tcp 127.0.0.1:19529 --key $KEY"
$CC rooms create a2a --encrypted --description "actor-to-actor" >/dev/null
ROOM=a2a; A=examples/actor-suite
spawn() { local seat=$1; shift; COWCHAT_WAKE_SECRET=w-$seat $CC --name "$seat" --agent-id "$seat" \
  actor-host "$ROOM" --listen 127.0.0.1:0 -- "$@" >"$DIR/$seat.log" 2>&1 & }
# addressed mode is default; actors mention each other by agent-id
spawn pinger  python3 "$A/pingpong_actor.py" pinger ponger
spawn ponger  python3 "$A/pingpong_actor.py" ponger pinger
spawn relay-a python3 "$A/relay_actor.py"    A relay-b
spawn relay-b python3 "$A/relay_actor.py"    B relay-c
spawn relay-c python3 "$A/relay_actor.py"    C END
sleep 1.5
$CC --name Chad --agent-id chad send "$ROOM" "3"     --mention pinger  >/dev/null
$CC --name Chad --agent-id chad send "$ROOM" "start" --mention relay-a >/dev/null
sleep 6
$CC --name Chad --agent-id chad history "$ROOM" --limit 200 > "$DIR/hist.txt" 2>&1
pass=0; fail=0; fails=()
check() { if grep -qF "$1" "$DIR/hist.txt"; then pass=$((pass+1)); else fail=$((fail+1)); fails+=("$2 (looked for '$1')"); fi; }
check "pinger: pinger 0"    "ping-pong decremented to 0"
check "ponger: done at ponger" "ping-pong terminated cleanly"
check "relay-c: A,B,C"      "relay forwarded A->B->C"
echo "=== A2A RESULT: $pass passed / $fail failed ==="
if [ $fail -gt 0 ]; then printf '  FAIL: %s\n' "${fails[@]}"; echo "--- history ---"; cat "$DIR/hist.txt"; echo "--- logs ---"; for s in pinger ponger relay-a relay-b relay-c; do echo "[$s]"; tail -3 "$DIR/$s.log"; done; fi
exit $([ $fail -eq 0 ] && echo 0 || echo 1)
