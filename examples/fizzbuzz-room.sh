#!/usr/bin/env bash
# Three-seat FizzBuzz room on a scratch local server: counter, fizz, and buzz
# actor hosts wake on mentions, run examples/python/seat_actor.py, and reply.
# Usage: cargo build -p cowchat-server -p cowchat-cli && examples/fizzbuzz-room.sh
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=target/debug
[ -x $BIN/cowchat-server ] && [ -x $BIN/cowchat ] || { echo "build first: cargo build -p cowchat-server -p cowchat-cli"; exit 1; }

DIR=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$DIR"' EXIT
export COWCHAT_ROOM_KEY=fizzbuzz-demo-secret

$BIN/cowchat-server serve --socket "$DIR/server.sock" --tcp 127.0.0.1:19229 \
  --db "$DIR/server.db" --key-file "$DIR/auth.key" --allow-private-webhooks >"$DIR/server.log" 2>&1 &
sleep 1
KEY=$(cat "$DIR/auth.key")
CC="$BIN/cowchat --tcp 127.0.0.1:19229 --key $KEY"

$CC rooms create fizzbuzz --description "3-seat FizzBuzz demo" >/dev/null
ROOM=fizzbuzz # the CLI resolves room names

port=19230
for seat in counter fizz buzz; do
  port=$((port + 1))
  COWCHAT_WAKE_SECRET=wake-$seat $CC --name $seat --agent-id $seat \
    actor-host "$ROOM" --listen 127.0.0.1:$port -- \
    python3 examples/python/seat_actor.py $seat >"$DIR/$seat.log" 2>&1 &
done
sleep 1

send() { $CC --name Chad --agent-id chad send "$ROOM" "$1" "${@:2}" >/dev/null; }
send 38 --mention counter
send 3  --mention fizz
send 5  --mention buzz
send 15 --mention fizz --mention buzz
sleep 3

echo "=== room history ==="
$CC --name Chad --agent-id chad history "$ROOM" --limit 20
