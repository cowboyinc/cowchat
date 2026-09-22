#!/usr/bin/env bash
# Local actor-messaging test suite: spawn many actor-host seats in one encrypted
# room, send addressed inputs, assert each reply. Fully local (no chain).
set -uo pipefail
cd "$(dirname "$0")/../.."
BIN=target/debug
[ -x $BIN/cowchat-server ] && [ -x $BIN/cowchat ] || { echo "build first: (cd cowchat && cargo build -p cowchat-server -p cowchat-cli)"; exit 2; }
DIR=$(mktemp -d); SUMDIR=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$DIR" "$SUMDIR"' EXIT
export COWCHAT_ROOM_KEY=actor-suite-secret SUM_STATE_DIR="$SUMDIR"
$BIN/cowchat-server serve --socket "$DIR/s.sock" --tcp 127.0.0.1:19329 \
  --db "$DIR/s.db" --key-file "$DIR/auth.key" --allow-private-webhooks >"$DIR/server.log" 2>&1 &
sleep 1.5
KEY=$(cat "$DIR/auth.key"); CC="$BIN/cowchat --tcp 127.0.0.1:19329 --key $KEY"
$CC rooms create suite --encrypted --description "actor-messaging suite" >/dev/null
ROOM=suite; A=examples/actor-suite; PY=examples/python
declare -A ACTOR=(
  [fizzbuzz]="$PY/fizzbuzz_actor.py" [upper]="$A/upper_actor.py" [reverse]="$A/reverse_actor.py"
  [wordcount]="$A/wordcount_actor.py" [rot13]="$A/rot13_actor.py" [calc]="$A/calc_actor.py"
  [morse]="$A/morse_actor.py" [magic8]="$A/magic8_actor.py" [keyword]="$A/keyword_actor.py" [sum]="$A/sum_actor.py")
port=19330
for seat in "${!ACTOR[@]}"; do
  port=$((port+1))
  COWCHAT_WAKE_SECRET=wake-$seat $CC --name "$seat" --agent-id "$seat" \
    actor-host "$ROOM" --listen 127.0.0.1:$port -- python3 ${ACTOR[$seat]} >"$DIR/$seat.log" 2>&1 &
done
sleep 1.5
send() { $CC --name bob --agent-id bob send "$ROOM" "$1" --mention "$2" >/dev/null; }
# magic8 expected computed from the actor's own hash so it stays reproducible
M8Q="will the suite pass"
M8EXP=$(python3 -c "import hashlib;A=['It is certain','Reply hazy, try again','Don\\'t count on it','Yes definitely','My sources say no','Signs point to yes','Outlook not so good','Without a doubt'];print(A[int(hashlib.sha256('$M8Q'.encode()).hexdigest(),16)%8])")
# cases: seat<TAB>input<TAB>expected  (order matters only for the stateful sum actor)
CASES=(
  $'fizzbuzz\t15\tFizzBuzz' $'fizzbuzz\t9\tFizz' $'fizzbuzz\t10\tBuzz' $'fizzbuzz\t7\t7'
  $'upper\thello world\tHELLO WORLD' $'reverse\tabcde\tedcba' $'wordcount\tone two three\t3'
  $'rot13\tHello\tUryyb' $'calc\t12 + 30\t42' $'calc\t7*6\t42' $'calc\t100 - 1\t99' $'calc\t84 / 2\t42'
  $'morse\tSOS\t... --- ...' $'keyword\ti love this\tpositive' $'keyword\tthis is bad\tnegative'
  $'keyword\tthe sky today\tneutral' $'sum\t10\t10' $'sum\t5\t15' $'sum\t100\t115')
CASES+=("$(printf 'magic8\t%s\t%s' "$M8Q" "$M8EXP")")
for c in "${CASES[@]}"; do IFS=$'\t' read -r seat inp exp <<<"$c"; send "$inp" "$seat"; sleep 0.15; done
sleep 5
$CC --name bob --agent-id bob history "$ROOM" --limit 300 > "$DIR/hist.txt" 2>&1
pass=0; fail=0; fails=()
for c in "${CASES[@]}"; do
  IFS=$'\t' read -r seat inp exp <<<"$c"
  # a history line ends with "  #N seat: <reply>"; match the exact reply for that seat
  if grep -qF "$seat: $exp" "$DIR/hist.txt"; then pass=$((pass+1)); else fail=$((fail+1)); fails+=("$seat <= '$inp'  EXPECTED '$exp'"); fi
done
echo "=== RESULT: $pass passed / $fail failed (of ${#CASES[@]}) ==="
if [ $fail -gt 0 ]; then echo "--- FAILURES ---"; printf '  %s\n' "${fails[@]}"; echo "--- history ---"; cat "$DIR/hist.txt"; echo "--- actor logs (non-empty) ---"; for seat in "${!ACTOR[@]}"; do [ -s "$DIR/$seat.log" ] && { echo "[$seat]"; tail -4 "$DIR/$seat.log"; }; done; fi
exit $([ $fail -eq 0 ] && echo 0 || echo 1)
