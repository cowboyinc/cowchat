#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
cd "$ROOT"

PRODUCT="CowchatMac"
APP_NAME="Cowchat.app"
ICON_SOURCE="$ROOT/Sources/CowchatMac/Resources/CowchatIcon.png"

swift build -c release
BIN_DIR=$(swift build -c release --show-bin-path)
APP_DIR="$BIN_DIR/$APP_NAME"
RESOURCE_BUNDLE="$BIN_DIR/${PRODUCT}_${PRODUCT}.bundle"

if [ ! -f "$BIN_DIR/$PRODUCT" ]; then
	printf '%s\n' "Missing release executable: $BIN_DIR/$PRODUCT" >&2
	exit 1
fi
if [ ! -d "$RESOURCE_BUNDLE" ]; then
	printf '%s\n' "Missing SwiftPM resource bundle: $RESOURCE_BUNDLE" >&2
	exit 1
fi

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$BIN_DIR/$PRODUCT" "$APP_DIR/Contents/MacOS/Cowchat"
cp "$ROOT/AppBundle/Info.plist" "$APP_DIR/Contents/Info.plist"
cp -R "$RESOURCE_BUNDLE" "$APP_DIR/Contents/Resources/"
cp "$ICON_SOURCE" "$APP_DIR/Contents/Resources/CowchatIcon.png"

ICONSET=$(mktemp -d "${TMPDIR:-/tmp}/CowchatIcon.XXXXXX.iconset")
cleanup() { rm -rf "$ICONSET"; }
trap cleanup EXIT

for size in 16 32 128 256 512; do
	sips -z "$size" "$size" "$ICON_SOURCE" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
	double=$((size * 2))
	sips -z "$double" "$double" "$ICON_SOURCE" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP_DIR/Contents/Resources/CowchatIcon.icns"

codesign --force --deep --sign - "$APP_DIR" >/dev/null

INSTALL_DIR="$HOME/Applications"
INSTALL_APP="$INSTALL_DIR/$APP_NAME"
mkdir -p "$INSTALL_DIR"
rm -rf "$INSTALL_APP"
ditto "$APP_DIR" "$INSTALL_APP"
codesign --force --deep --sign - "$INSTALL_APP" >/dev/null

printf 'Built: %s\n' "$APP_DIR"
printf 'Installed: %s\n' "$INSTALL_APP"
