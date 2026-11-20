#!/bin/bash
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: live-test.sh <path-to-cowchat-cli> <api-key-file> [tcp-port]" >&2
  exit 64
fi
cli=$1
key=$(cat "$2")
port=${3:-9339}
nonce=$(date +%s)
cd "$(dirname "$0")/.."
peer() { "$cli" --tcp "127.0.0.1:$port" --key "$key" --name pinger --agent-id pinger-1 "$@"; }

xcodegen generate >/dev/null
# An invite to a room named "invited", for the add-by-invite and remove steps.
peer rooms create invited >/dev/null 2>&1 || true
invite=$(peer invites create invited --open 2>/dev/null | grep -oE 'cinv_[A-Za-z0-9_-]+' | head -1 || true)
markdown=$'Review of **PR 12**:\n\n```swift\nlet x = timeline.merge(page)\n```'
long=$'long report line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nlast line of the long report'
fixtures=$(mktemp -d)
python3 -I - "$fixtures" <<'PY'
import json, struct, sys, zlib
def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
rows = b"".join(b"\x00" + bytes([230, 120, 40] * 64) for _ in range(32))
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 64, 32, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")
open(sys.argv[1] + "/chart.png", "wb").write(png)
json.dump({"repository": "cowboyinc/cowchat", "number": 12, "title": "Fold agent messages", "checks": "passing",
           "additions": 214, "deletions": 38, "address": "https://github.com/cowboyinc/cowchat/pull/12"},
          open(sys.argv[1] + "/pr.json", "w"))
PY
(
  n=0
  # The fixtures folder goes when the script exits, which also stops a sender that outlived it.
  while [ -d "$fixtures" ]; do
    n=$((n+1))
    peer send design-review "live ping $nonce-$n" >/dev/null 2>&1 || true
    if (( n % 4 == 0 )); then peer send design-review "$markdown" >/dev/null 2>&1 || true; fi
    if (( n % 4 == 2 )); then peer send design-review "$long" >/dev/null 2>&1 || true; fi
    # The server allows 60 charts an hour for one key, so charts go out sparingly.
    if (( n % 12 == 1 )); then peer send-chart --title "Latency by hour" design-review "$fixtures/chart.png" >/dev/null 2>&1 || true; fi
    if (( n % 4 == 3 )); then peer send design-review "opened a pull request" --pull-request "$fixtures/pr.json" >/dev/null 2>&1 || true; fi
    peer thinking design-review "reading the diff" >/dev/null 2>&1 || true
    sleep 3
  done
) &
pinger=$!
# A room made while the phone is connected, to show the list updates without a refresh.
( sleep 70; peer rooms create "late-$nonce" >/dev/null 2>&1 || true ) &
late=$!
# Mention suggestions come from the room's connected agents, so one peer stays online.
( while true; do "$cli" --tcp "127.0.0.1:$port" --key "$key" --name ada-agent --agent-id ada-online wait design-review >/dev/null 2>&1 || sleep 1; done ) &
online=$!
trap 'rm -rf "$fixtures"; kill $pinger $online $late 2>/dev/null || true; pkill -f "agent-id ada-online" 2>/dev/null || true' EXIT

TEST_RUNNER_COWCHAT_TEST_KEY=$key TEST_RUNNER_COWCHAT_TEST_PORT=$port TEST_RUNNER_COWCHAT_TEST_NONCE=$nonce TEST_RUNNER_COWCHAT_TEST_INVITE=$invite \
  xcodebuild test -project CowchatiOS.xcodeproj -scheme CowchatiOSLive \
  -destination "platform=iOS Simulator,name=${COWCHAT_TEST_DEVICE:-iPhone 17}" \
  -derivedDataPath build-live 2>&1 \
  | grep -E "error:|: error|XCTAssert|failed|Executed|PROMPT_DISMISSED|TEST (SUCCEEDED|FAILED)" || true

peer rooms destroy --yes "late-$nonce" >/dev/null 2>&1 || true
peer history "phone-$nonce-2" | grep -q "first post $nonce" && echo "PEER_SAW_PHONE_ROOM: yes" || echo "PEER_SAW_PHONE_ROOM: no"
peer history design-review --limit 400 | grep -q "hello from phone $nonce" && echo "PEER_SAW_PHONE_MESSAGE: yes" || echo "PEER_SAW_PHONE_MESSAGE: no"
peer --room-key 'correct horse battery' history sealed | grep -q "sealed reply $nonce" && echo "PEER_DECRYPTED_SEALED_REPLY: yes" || echo "PEER_DECRYPTED_SEALED_REPLY: no"
