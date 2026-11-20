#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
# Debug builds are the "Cowchat Dev" app.
BUNDLE_ID=inc.cowboy.cowchat.ios.dev
DERIVED=build-device

json=$(mktemp)
trap 'rm -f "$json"' EXIT
xcrun devicectl list devices --json-output "$json" >/dev/null 2>&1 || true

device=$(python3 -I - "$json" <<'PY'
import json, sys
try:
    devices = json.load(open(sys.argv[1]))["result"]["devices"]
except Exception:
    devices = []
for d in devices:
    hw = d.get("hardwareProperties", {})
    conn = d.get("connectionProperties", {})
    if hw.get("deviceType") == "iPhone" and hw.get("reality") == "physical" and conn.get("pairingState") == "paired":
        print(d["identifier"], hw["udid"])
        break
PY
)

if [ -z "$device" ]; then
  echo "No iPhone found: connect and trust an iPhone, then run again." >&2
  exit 1
fi
read -r device udid <<<"$device"

xcodegen generate
xcodebuild build \
  -project CowchatiOS.xcodeproj \
  -scheme CowchatiOS \
  -configuration Debug \
  -destination "id=$udid" \
  -derivedDataPath "$DERIVED" \
  -allowProvisioningUpdates

app="$DERIVED/Build/Products/Debug-iphoneos/CowchatiOS.app"
xcrun devicectl device install app --device "$device" "$app"
xcrun devicectl device process launch --device "$device" --terminate-existing "$BUNDLE_ID"
