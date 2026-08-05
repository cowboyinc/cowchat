#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname "$0")" && pwd)
cd "$ROOT"

PRODUCT="CowchatMac"
APP_NAME="Cowchat.app"
ICON_SOURCE="$ROOT/Sources/CowchatMac/Resources/CowchatIcon.png"
ICON_PACKAGE="$ROOT/Cowchat.icon"
SIGNING_IDENTITY=${COWCHAT_CODESIGN_IDENTITY:--}

if [ ! -d "$ICON_PACKAGE" ]; then
	printf 'Missing Icon Composer source: %s\n' "$ICON_PACKAGE" >&2
	exit 1
fi
if ! cmp -s "$ICON_SOURCE" "$ICON_PACKAGE/Assets/icon 2.png"; then
	printf '%s\n' 'The SwiftPM fallback icon does not match the Icon Composer source.' >&2
	exit 1
fi
if ! command -v xcrun >/dev/null 2>&1; then
	printf '%s\n' 'Missing xcrun; Xcode is required to compile Cowchat.icon.' >&2
	exit 1
fi

# Build both supported Mac architectures so a versioned installer is never
# silently tied to the architecture of the machine that produced it.
swift build -c release --arch arm64 --arch x86_64
BIN_DIR=$(swift build -c release --arch arm64 --arch x86_64 --show-bin-path)
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
if ! lipo "$BIN_DIR/$PRODUCT" -verify_arch arm64 x86_64; then
	printf '%s\n' 'Release executable is not universal (arm64 + x86_64).' >&2
	exit 1
fi

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$BIN_DIR/$PRODUCT" "$APP_DIR/Contents/MacOS/Cowchat"
cp "$ROOT/AppBundle/Info.plist" "$APP_DIR/Contents/Info.plist"
cp -R "$RESOURCE_BUNDLE" "$APP_DIR/Contents/Resources/"

TMP_BASE=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
ICON_BUILD=$(mktemp -d "$TMP_BASE/CowchatIcon.XXXXXX")
INSTALL_DIR=''
INSTALL_APP=''
INSTALL_STAGE=''
INSTALL_BACKUP=''
INSTALL_SWAP_PENDING=0

cleanup() {
	cleanup_status=$?
	trap - EXIT HUP INT TERM
	cleanup_failed=0
	if [ "$INSTALL_SWAP_PENDING" -eq 1 ]; then
		if [ ! -e "$INSTALL_APP" ] && [ ! -L "$INSTALL_APP" ] && [ -e "$INSTALL_BACKUP" ]; then
			if mv "$INSTALL_BACKUP" "$INSTALL_APP"; then
				INSTALL_SWAP_PENDING=0
			else
				cleanup_failed=1
			fi
		else
			cleanup_failed=1
		fi
		if [ "$cleanup_failed" -ne 0 ]; then
			printf 'Could not restore the previous Cowchat.app; recovery files remain at: %s\n' "$INSTALL_STAGE" >&2
		fi
	fi
	if [ -n "$INSTALL_STAGE" ] && [ "$INSTALL_SWAP_PENDING" -eq 0 ]; then
		case $INSTALL_STAGE in
			"$INSTALL_DIR"/.cowchat-install.*)
				if ! /bin/rm -rf -- "$INSTALL_STAGE"; then cleanup_failed=1; fi
				;;
			*)
				printf 'Refusing to clean unexpected install staging path: %s\n' "$INSTALL_STAGE" >&2
				cleanup_failed=1
				;;
		esac
	fi
	case $ICON_BUILD in
		"$TMP_BASE"/CowchatIcon.*)
			if ! /bin/rm -rf -- "$ICON_BUILD"; then cleanup_failed=1; fi
			;;
		*)
			printf 'Refusing to clean unexpected icon staging path: %s\n' "$ICON_BUILD" >&2
			cleanup_failed=1
			;;
	esac
	if [ "$cleanup_status" -eq 0 ] && [ "$cleanup_failed" -ne 0 ]; then
		cleanup_status=1
	fi
	exit "$cleanup_status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir "$ICON_BUILD/out"
xcrun actool \
	--compile "$ICON_BUILD/out" \
	--platform macosx \
	--minimum-deployment-target 13.0 \
	--target-device mac \
	--app-icon Cowchat \
	--standalone-icon-behavior all \
	--output-partial-info-plist "$ICON_BUILD/partial.plist" \
	--output-format human-readable-text \
	"$ICON_PACKAGE" >/dev/null
cp "$ICON_BUILD/out/Assets.car" "$APP_DIR/Contents/Resources/Assets.car"
cp "$ICON_BUILD/out/Cowchat.icns" "$APP_DIR/Contents/Resources/Cowchat.icns"

if [ ! -s "$APP_DIR/Contents/Resources/Assets.car" ] || \
	[ ! -s "$APP_DIR/Contents/Resources/Cowchat.icns" ]; then
	printf '%s\n' 'Icon Composer did not produce the expected app icon artifacts.' >&2
	exit 1
fi

if [ "$SIGNING_IDENTITY" = "-" ]; then
	codesign --force --deep --sign - "$APP_DIR" >/dev/null
else
	codesign --force --deep --options runtime --timestamp --sign "$SIGNING_IDENTITY" "$APP_DIR" >/dev/null
fi
codesign --verify --deep --strict "$APP_DIR"

INSTALL_DIR="$HOME/Applications"
mkdir -p "$INSTALL_DIR"
INSTALL_DIR=$(CDPATH='' cd -- "$INSTALL_DIR" && pwd)
INSTALL_APP="$INSTALL_DIR/$APP_NAME"
INSTALL_STAGE=$(mktemp -d "$INSTALL_DIR/.cowchat-install.XXXXXX")
INSTALL_CANDIDATE="$INSTALL_STAGE/$APP_NAME"
ditto "$APP_DIR" "$INSTALL_CANDIDATE"
codesign --verify --deep --strict "$INSTALL_CANDIDATE"
if ! lipo "$INSTALL_CANDIDATE/Contents/MacOS/Cowchat" -verify_arch arm64 x86_64; then
	printf '%s\n' 'The staged installed app is not universal (arm64 + x86_64).' >&2
	exit 1
fi

if [ -e "$INSTALL_APP" ] || [ -L "$INSTALL_APP" ]; then
	INSTALL_BACKUP="$INSTALL_STAGE/previous-$APP_NAME"
	INSTALL_SWAP_PENDING=1
	if ! mv "$INSTALL_APP" "$INSTALL_BACKUP"; then
		# rename(2) is atomic. If the original still exists and no backup was
		# created, there is nothing for cleanup to roll back.
		if { [ -e "$INSTALL_APP" ] || [ -L "$INSTALL_APP" ]; } && \
			[ ! -e "$INSTALL_BACKUP" ] && [ ! -L "$INSTALL_BACKUP" ]; then
			INSTALL_SWAP_PENDING=0
		fi
		printf '%s\n' 'Could not move the previous Cowchat.app into rollback staging.' >&2
		exit 1
	fi
fi
if ! mv "$INSTALL_CANDIDATE" "$INSTALL_APP"; then
	printf '%s\n' 'Could not publish the verified Cowchat.app candidate.' >&2
	exit 1
fi
INSTALL_SWAP_PENDING=0
/bin/rm -rf -- "$INSTALL_STAGE"
INSTALL_STAGE=''

printf 'Built: %s\n' "$APP_DIR"
printf 'Installed: %s\n' "$INSTALL_APP"
