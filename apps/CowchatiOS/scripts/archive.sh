#!/usr/bin/env bash
set -euo pipefail

if [ $# -ne 1 ] || ! [[ "$1" =~ ^[0-9]+$ ]]; then
  echo "usage: scripts/archive.sh <build-number>" >&2
  exit 2
fi
build_number=$1

cd "$(dirname "$0")/.."
archive=build/Cowchat.xcarchive
export_dir=build/export

xcodegen generate
rm -rf "$archive" "$export_dir"

xcodebuild archive \
  -project CowchatiOS.xcodeproj \
  -scheme CowchatiOS \
  -configuration Release \
  -destination "generic/platform=iOS" \
  -archivePath "$archive" \
  -derivedDataPath build-archive \
  -allowProvisioningUpdates \
  CURRENT_PROJECT_VERSION="$build_number"

xcodebuild -exportArchive \
  -archivePath "$archive" \
  -exportPath "$export_dir" \
  -exportOptionsPlist scripts/ExportOptions.plist \
  -allowProvisioningUpdates

ipa=$(ls "$export_dir"/*.ipa)
echo "$PWD/$ipa"
