#!/bin/sh
set -eu
ROOT=$(CDPATH='' cd -- "$(dirname "$0")" && pwd)
FRAMEWORK=${1:?usage: test-sparkle-packaging.sh path/to/Sparkle.framework}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/cowchat-sparkle-packaging.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
# This test only uses an ad-hoc identity, never a production credential.
ditto "$FRAMEWORK" "$WORK/Sparkle.framework"
for service in Installer Downloader; do
    codesign -d --entitlements :- "$WORK/Sparkle.framework/Versions/B/XPCServices/$service.xpc" \
        > "$WORK/$service-before.plist" 2>/dev/null
    test -s "$WORK/$service-before.plist"
done
. "$ROOT/sign-sparkle.sh"
sign_sparkle_framework "$WORK/Sparkle.framework" -
for service in Installer Downloader; do
    codesign -d --entitlements :- "$WORK/Sparkle.framework/Versions/B/XPCServices/$service.xpc" \
        > "$WORK/$service-after.plist" 2>/dev/null
    cmp "$WORK/$service-before.plist" "$WORK/$service-after.plist"
done
test -L "$WORK/Sparkle.framework/Sparkle"
test -L "$WORK/Sparkle.framework/Versions/Current"
lipo "$WORK/Sparkle.framework/Versions/B/Sparkle" -verify_arch arm64
printf '%s\n' 'Sparkle nested signatures, preserved entitlements, arm64 slice and framework symlinks verified.'
