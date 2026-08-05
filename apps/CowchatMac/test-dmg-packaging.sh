#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname "$0")" && pwd)
TEST_TMP_BASE=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
WORK=$(mktemp -d "$TEST_TMP_BASE/cowchat-dmg-test.XXXXXX")

cleanup() {
	case $WORK in
		"$TEST_TMP_BASE"/cowchat-dmg-test.*) /bin/rm -rf -- "$WORK" ;;
		*) printf 'Refusing to clean unexpected temporary path: %s\n' "$WORK" >&2 ;;
	esac
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

sh -n "$ROOT/build-dmg.sh"
sh -n "$ROOT/build-app.sh"
xmllint --noout "$ROOT/DmgBackground.svg"
sips -s format png "$ROOT/DmgBackground.svg" --out "$WORK/background.png" >/dev/null

if ! cmp -s "$ROOT/Sources/CowchatMac/Resources/CowchatIcon.png" "$ROOT/Cowchat.icon/Assets/icon 2.png"; then
	printf '%s\n' 'SwiftPM and Icon Composer icon sources differ.' >&2
	exit 1
fi
mkdir "$WORK/icon-out"
xcrun actool \
	--compile "$WORK/icon-out" \
	--platform macosx \
	--minimum-deployment-target 13.0 \
	--target-device mac \
	--app-icon Cowchat \
	--standalone-icon-behavior all \
	--output-partial-info-plist "$WORK/icon-info.plist" \
	--output-format human-readable-text \
	"$ROOT/Cowchat.icon" >/dev/null
if [ ! -s "$WORK/icon-out/Assets.car" ] || [ ! -s "$WORK/icon-out/Cowchat.icns" ]; then
	printf '%s\n' 'Icon Composer packaging check failed.' >&2
	exit 1
fi
if [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIconName' "$WORK/icon-info.plist")" != "Cowchat" ]; then
	printf '%s\n' 'Icon Composer emitted an unexpected bundle icon name.' >&2
	exit 1
fi

WIDTH=$(sips -g pixelWidth "$WORK/background.png" | awk '/pixelWidth:/ { print $2 }')
HEIGHT=$(sips -g pixelHeight "$WORK/background.png" | awk '/pixelHeight:/ { print $2 }')
if [ "$WIDTH" != "660" ] || [ "$HEIGHT" != "420" ]; then
	printf 'Unexpected background size: %sx%s\n' "$WIDTH" "$HEIGHT" >&2
	exit 1
fi

grep -q 'codesign --verify --deep --strict' "$ROOT/build-dmg.sh"
grep -Fq "attach_image \"\$FINAL_DMG\" -readonly" "$ROOT/build-dmg.sh"
grep -q 'Applications.*symlink to /Applications' "$ROOT/build-dmg.sh"
grep -q 'with timeout of 30 seconds' "$ROOT/build-dmg.sh"
grep -q 'Finder automation is unavailable' "$ROOT/build-dmg.sh"
grep -q 'lipo .* -verify_arch arm64 x86_64' "$ROOT/build-app.sh"
grep -q 'lipo .* -verify_arch arm64 x86_64' "$ROOT/build-dmg.sh"
grep -q 'xcrun actool' "$ROOT/build-app.sh"
grep -q -- '--standalone-icon-behavior all' "$ROOT/build-app.sh"
grep -q '<string>Cowchat</string>' "$ROOT/AppBundle/Info.plist"
grep -Fq "hdiutil verify \"\$TEMP_OUTPUT\"" "$ROOT/build-dmg.sh"
grep -Fq "mv -f \"\$TEMP_OUTPUT\" \"\$OUTPUT_DMG\"" "$ROOT/build-dmg.sh"
grep -q 'notarytool submit' "$ROOT/build-dmg.sh"
grep -q 'stapler validate' "$ROOT/build-dmg.sh"
grep -q '#F9F7F5' "$ROOT/DmgBackground.svg"
grep -q '#FF9D14' "$ROOT/DmgBackground.svg"

final_verify_line=$(grep -nF "hdiutil verify \"\$TEMP_OUTPUT\"" "$ROOT/build-dmg.sh" | tail -n 1 | cut -d: -f1)
staple_line=$(grep -nF "xcrun stapler staple \"\$TEMP_OUTPUT\"" "$ROOT/build-dmg.sh" | cut -d: -f1)
dmg_publish_line=$(grep -nF "mv -f \"\$TEMP_OUTPUT\" \"\$OUTPUT_DMG\"" "$ROOT/build-dmg.sh" | cut -d: -f1)
if [ "$final_verify_line" -le "$staple_line" ] || [ "$final_verify_line" -ge "$dmg_publish_line" ]; then
	printf '%s\n' 'The final DMG integrity check must follow stapling and precede publication.' >&2
	exit 1
fi

install_copy_line=$(grep -nF "ditto \"\$APP_DIR\" \"\$INSTALL_CANDIDATE\"" "$ROOT/build-app.sh" | cut -d: -f1)
install_verify_line=$(grep -nF "codesign --verify --deep --strict \"\$INSTALL_CANDIDATE\"" "$ROOT/build-app.sh" | cut -d: -f1)
install_backup_line=$(grep -nF "mv \"\$INSTALL_APP\" \"\$INSTALL_BACKUP\"" "$ROOT/build-app.sh" | cut -d: -f1)
install_publish_line=$(grep -nF "mv \"\$INSTALL_CANDIDATE\" \"\$INSTALL_APP\"" "$ROOT/build-app.sh" | cut -d: -f1)
if [ "$install_copy_line" -ge "$install_verify_line" ] || \
	[ "$install_verify_line" -ge "$install_backup_line" ] || \
	[ "$install_backup_line" -ge "$install_publish_line" ]; then
	printf '%s\n' 'The app candidate must be copied and verified before the installed app is moved.' >&2
	exit 1
fi

# Prove that a failed candidate copy cannot destroy or partially overwrite an
# existing installation. These shims isolate the atomic install transaction
# from the real Swift build and code-signing tools.
APP_TEST="$WORK/app-install-failure"
APP_SHIMS="$APP_TEST/shims"
APP_BIN="$APP_TEST/swift-bin"
APP_HOME="$APP_TEST/home"
mkdir -p "$APP_SHIMS" "$APP_BIN/CowchatMac_CowchatMac.bundle" \
	"$APP_HOME/Applications/Cowchat.app/Contents"
printf '%s\n' 'previous-install' > "$APP_HOME/Applications/Cowchat.app/Contents/sentinel"
printf '%s\n' 'fake-universal-binary' > "$APP_BIN/CowchatMac"

cat > "$APP_SHIMS/swift" <<'SHIM'
#!/bin/sh
case " $* " in
	*" --show-bin-path "*) printf '%s\n' "$COWCHAT_TEST_BIN_DIR" ;;
esac
SHIM
cat > "$APP_SHIMS/lipo" <<'SHIM'
#!/bin/sh
exit 0
SHIM
cat > "$APP_SHIMS/codesign" <<'SHIM'
#!/bin/sh
exit 0
SHIM
cat > "$APP_SHIMS/xcrun" <<'SHIM'
#!/bin/sh
shift
output_dir=''
partial_plist=''
while [ "$#" -gt 0 ]; do
	case $1 in
		--compile) shift; output_dir=$1 ;;
		--output-partial-info-plist) shift; partial_plist=$1 ;;
	esac
	shift
done
mkdir -p "$output_dir"
printf '%s\n' 'assets' > "$output_dir/Assets.car"
printf '%s\n' 'icon' > "$output_dir/Cowchat.icns"
printf '%s\n' '<?xml version="1.0"?><plist version="1.0"><dict/></plist>' > "$partial_plist"
SHIM
cat > "$APP_SHIMS/ditto" <<'SHIM'
#!/bin/sh
if [ "${COWCHAT_TEST_FAIL_DITTO:-0}" = "1" ]; then
	mkdir -p "$2"
	printf '%s\n' 'partial-copy' > "$2/partial"
	exit 73
fi
cp -R "$1" "$2"
SHIM
cat > "$APP_SHIMS/mv" <<'SHIM'
#!/bin/sh
case ${2:-} in
	*/previous-Cowchat.app)
		if [ "${COWCHAT_TEST_SIGNAL_AFTER_BACKUP:-0}" = "1" ]; then
			/bin/mv "$@"
			kill -TERM "$PPID"
			exit 0
		fi
		;;
esac
exec /bin/mv "$@"
SHIM
chmod +x "$APP_SHIMS/swift" "$APP_SHIMS/lipo" "$APP_SHIMS/codesign" \
	"$APP_SHIMS/xcrun" "$APP_SHIMS/ditto" "$APP_SHIMS/mv"

if /usr/bin/env \
	HOME="$APP_HOME" \
	PATH="$APP_SHIMS:/usr/bin:/bin:/usr/sbin:/sbin" \
	COWCHAT_TEST_BIN_DIR="$APP_BIN" \
	COWCHAT_TEST_FAIL_DITTO=1 \
	"$ROOT/build-app.sh" > "$APP_TEST/build.log" 2>&1; then
	printf '%s\n' 'The injected app copy failure unexpectedly succeeded.' >&2
	exit 1
fi
if [ "$(cat "$APP_HOME/Applications/Cowchat.app/Contents/sentinel")" != "previous-install" ]; then
	printf '%s\n' 'A failed app copy did not preserve the prior installation.' >&2
	exit 1
fi
if find "$APP_HOME/Applications" -maxdepth 1 -name '.cowchat-install.*' | grep -q .; then
	printf '%s\n' 'A failed app copy leaked its install staging directory.' >&2
	exit 1
fi

if /usr/bin/env \
	HOME="$APP_HOME" \
	PATH="$APP_SHIMS:/usr/bin:/bin:/usr/sbin:/sbin" \
	COWCHAT_TEST_BIN_DIR="$APP_BIN" \
	COWCHAT_TEST_SIGNAL_AFTER_BACKUP=1 \
	"$ROOT/build-app.sh" > "$APP_TEST/signal-after-backup.log" 2>&1; then
	printf '%s\n' 'The injected signal after backup unexpectedly succeeded.' >&2
	exit 1
fi
if [ "$(cat "$APP_HOME/Applications/Cowchat.app/Contents/sentinel")" != "previous-install" ]; then
	printf '%s\n' 'An interrupted app swap did not restore the prior installation.' >&2
	exit 1
fi
if find "$APP_HOME/Applications" -maxdepth 1 -name '.cowchat-install.*' | grep -q .; then
	printf '%s\n' 'An interrupted app swap leaked its install staging directory.' >&2
	exit 1
fi

# Simulate hdiutil returning an error after allocating a device. Cleanup must
# resolve that device from hdiutil info. It may delete work only after detach;
# if detach also fails, it must preserve the backing directory for recovery.
DMG_TEST="$WORK/dmg-attach-failure"
DMG_SHIMS="$DMG_TEST/shims"
DMG_APP="$DMG_TEST/Cowchat.app"
DMG_TMP="$DMG_TEST/tmp"
DMG_OUTPUT="$DMG_TEST/output"
mkdir -p "$DMG_SHIMS" "$DMG_APP/Contents/MacOS" "$DMG_TMP" "$DMG_OUTPUT"
cp "$ROOT/AppBundle/Info.plist" "$DMG_APP/Contents/Info.plist"
printf '%s\n' 'fake-universal-binary' > "$DMG_APP/Contents/MacOS/Cowchat"

cat > "$DMG_SHIMS/codesign" <<'SHIM'
#!/bin/sh
case " $* " in
	*" -dv "*) printf '%s\n' 'Signature=adhoc' >&2 ;;
esac
exit 0
SHIM
cat > "$DMG_SHIMS/lipo" <<'SHIM'
#!/bin/sh
exit 0
SHIM
cat > "$DMG_SHIMS/osascript" <<'SHIM'
#!/bin/sh
exit 0
SHIM
cat > "$DMG_SHIMS/ditto" <<'SHIM'
#!/bin/sh
cp -R "$1" "$2"
SHIM
cat > "$DMG_SHIMS/sips" <<'SHIM'
#!/bin/sh
case " $* " in
	*" pixelWidth "*) printf '%s\n' '  pixelWidth: 660'; exit 0 ;;
	*" pixelHeight "*) printf '%s\n' '  pixelHeight: 420'; exit 0 ;;
esac
output=''
while [ "$#" -gt 0 ]; do
	if [ "$1" = "--out" ]; then shift; output=$1; fi
	shift
done
[ -z "$output" ] || printf '%s\n' 'fake-png' > "$output"
SHIM
cat > "$DMG_SHIMS/hdiutil" <<'SHIM'
#!/bin/sh
command_name=$1
shift
case $command_name in
	create)
		for argument do output_path=$argument; done
		printf '%s\n' 'fake-dmg' > "$output_path"
		;;
	attach)
		printf '%s\n' "$1" > "$COWCHAT_HDI_STATE/image-path"
		exit 74
		;;
	info)
		image_path=$(cat "$COWCHAT_HDI_STATE/image-path")
		cat <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>images</key><array><dict>
<key>image-path</key><string>$image_path</string>
<key>system-entities</key><array><dict>
<key>dev-entry</key><string>/dev/disk999</string>
</dict></array></dict></array></dict></plist>
PLIST
		;;
	detach)
		printf '%s\n' "$1" >> "$COWCHAT_HDI_STATE/detach-attempts"
		if [ "${COWCHAT_TEST_DETACH_FAIL:-0}" = "1" ]; then exit 75; fi
		;;
	*) exit 76 ;;
esac
SHIM
chmod +x "$DMG_SHIMS/codesign" "$DMG_SHIMS/lipo" "$DMG_SHIMS/osascript" \
	"$DMG_SHIMS/ditto" "$DMG_SHIMS/sips" "$DMG_SHIMS/hdiutil"

run_partial_attach_test() {
	test_name=$1
	detach_should_fail=$2
	state_dir="$DMG_TEST/state-$test_name"
	log_file="$DMG_TEST/$test_name.log"
	mkdir "$state_dir"
	printf '%s\n' 'previous-image' > "$DMG_OUTPUT/Cowchat-0.5.1.dmg"
	if /usr/bin/env \
		TMPDIR="$DMG_TMP" \
		PATH="$DMG_SHIMS:/usr/bin:/bin:/usr/sbin:/sbin" \
		COWCHAT_DMG_OUTPUT_DIR="$DMG_OUTPUT" \
		COWCHAT_HDI_STATE="$state_dir" \
		COWCHAT_TEST_DETACH_FAIL="$detach_should_fail" \
		"$ROOT/build-dmg.sh" "$DMG_APP" > "$log_file" 2>&1; then
		printf 'The injected partial attach failure unexpectedly succeeded (%s).\n' "$test_name" >&2
		exit 1
	fi
	if ! grep -Fxq '/dev/disk999' "$state_dir/detach-attempts"; then
		printf 'Cleanup did not resolve and detach the allocated device (%s).\n' "$test_name" >&2
		exit 1
	fi
	if [ "$(cat "$DMG_OUTPUT/Cowchat-0.5.1.dmg")" != "previous-image" ]; then
		printf 'A failed DMG build overwrote the previous image (%s).\n' "$test_name" >&2
		exit 1
	fi
}

run_partial_attach_test detach-recovers 0
if find "$DMG_TMP" -maxdepth 1 -type d -name 'cowchat-dmg.*' | grep -q .; then
	printf '%s\n' 'A successfully detached partial attach leaked recovery work.' >&2
	exit 1
fi

run_partial_attach_test detach-fails 1
recovery_dir=$(find "$DMG_TMP" -maxdepth 1 -type d -name 'cowchat-dmg.*' | head -n 1)
if [ -z "$recovery_dir" ] || ! grep -Fq 'Preserving recovery files at:' "$DMG_TEST/detach-fails.log"; then
	printf '%s\n' 'A detach failure did not preserve its recovery directory.' >&2
	exit 1
fi

printf '%s\n' 'DMG packaging safety checks passed.'
