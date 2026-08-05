#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname "$0")" && pwd)
TEST_TMP_BASE=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
WORK=$(mktemp -d "$TEST_TMP_BASE/cowchat-dmg-test.XXXXXX")

# Safety tests must never inherit release signing or notarization credentials.
unset COWCHAT_CODESIGN_IDENTITY COWCHAT_NOTARY_PROFILE COWCHAT_EXPECTED_TEAM_ID

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
grep -Fq "Contents/Helpers/\$SERVER_NAME" "$ROOT/build-app.sh"
grep -Fq "Contents/Helpers/\$SERVER_NAME" "$ROOT/build-dmg.sh"
grep -Fq "\"\$CARGO_BIN\" build --locked --release --package \"\$SERVER_NAME\"" "$ROOT/build-app.sh"
grep -Fq 'MACOSX_DEPLOYMENT_TARGET=13.0' "$ROOT/build-app.sh"
grep -Fq 'aarch64-apple-darwin x86_64-apple-darwin' "$ROOT/build-app.sh"
grep -Fq 'Cowchat app/server version mismatch' "$ROOT/build-app.sh"
grep -Fq "verify_server_binary \"\$INSTALL_SERVER_HELPER\" \"staged installed app\"" "$ROOT/build-app.sh"
grep -Fq "verify_server_signature \"\$INSTALL_SERVER_HELPER\" \"staged installed app\"" "$ROOT/build-app.sh"
grep -Fq "verify_server_helper \"\$volume_root/Cowchat.app\" \"\$verification_phase image\"" "$ROOT/build-dmg.sh"
grep -Fq "verify_payload \"\$MOUNT_POINT\" writable" "$ROOT/build-dmg.sh"
grep -Fq "verify_payload \"\$MOUNT_POINT\" read-only" "$ROOT/build-dmg.sh"
grep -Fq "verify_dmg_signer_matches_app \"\$DMG_SIGNATURE_INFO\"" "$ROOT/build-dmg.sh"
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

server_lipo_line=$(grep -nF "lipo \"\$ARM_SERVER\" \"\$INTEL_SERVER\" -create -output \"\$SERVER_HELPER\"" "$ROOT/build-app.sh" | cut -d: -f1)
server_sign_line=$(grep -nF "codesign --force --sign - \"\$SERVER_HELPER\"" "$ROOT/build-app.sh" | cut -d: -f1)
app_sign_line=$(grep -nF "codesign --force --sign - \"\$APP_DIR\"" "$ROOT/build-app.sh" | cut -d: -f1)
if [ "$server_lipo_line" -ge "$server_sign_line" ] || [ "$server_sign_line" -ge "$app_sign_line" ]; then
	printf '%s\n' 'The universal server helper must be assembled and signed before the outer app.' >&2
	exit 1
fi

# Exercise the exact production signer-chain function without needing a real
# Finder mount or Developer ID credential. These regressions specifically keep
# EXPECTED_TEAM_ID empty: the app identity itself must still bind the DMG.
SIGNER_FUNCTION="$WORK/verify-dmg-signer.sh"
sed -n '/^verify_dmg_signer_matches_app() {/,/^}/p' "$ROOT/build-dmg.sh" > "$SIGNER_FUNCTION"
if [ ! -s "$SIGNER_FUNCTION" ]; then
	printf '%s\n' 'Could not extract the DMG signer-chain verifier.' >&2
	exit 1
fi

run_dmg_signer_test() {
	test_name=$1
	dmg_team=$2
	dmg_authority=$3
	should_pass=$4
	expected_error=${5:-}
	log_file="$WORK/dmg-signer-$test_name.log"
	dmg_signature_info=$(printf '%s\n' "$dmg_authority" "TeamIdentifier=$dmg_team")
	if /usr/bin/env \
		APP_IS_DEVELOPER_ID=1 \
		APP_TEAM_ID=COWBOY123 \
		APP_DEVELOPER_AUTHORITY='Authority=Developer ID Application: Cowboy Inc. (COWBOY123)' \
		EXPECTED_TEAM_ID='' \
		NOTARY_PROFILE='' \
		DMG_SIGNATURE_INFO="$dmg_signature_info" \
		/bin/sh -c ". \"\$1\"; verify_dmg_signer_matches_app \"\$DMG_SIGNATURE_INFO\"" \
		sh "$SIGNER_FUNCTION" > "$log_file" 2>&1; then
		actual_result=pass
	else
		actual_result=fail
	fi
	if [ "$actual_result" != "$should_pass" ]; then
		printf 'Unexpected DMG signer-chain result (%s): expected=%s actual=%s.\n' \
			"$test_name" "$should_pass" "$actual_result" >&2
		exit 1
	fi
	if [ -n "$expected_error" ] && ! grep -Fq "$expected_error" "$log_file"; then
		printf 'DMG signer-chain failure was not actionable (%s).\n' "$test_name" >&2
		exit 1
	fi
}

run_dmg_signer_test matching COWBOY123 \
	'Authority=Developer ID Application: Cowboy Inc. (COWBOY123)' pass
run_dmg_signer_test wrong-team OTHERTEAM \
	'Authority=Developer ID Application: Cowboy Inc. (COWBOY123)' fail \
	'Cowchat DMG TeamIdentifier does not match the app: dmg=OTHERTEAM app=COWBOY123.'
run_dmg_signer_test wrong-authority COWBOY123 \
	'Authority=Developer ID Application: Other Corp (COWBOY123)' fail \
	'Cowchat DMG signing authority does not match the app.'

# Prove that a failed candidate copy cannot destroy or partially overwrite an
# existing installation. These shims isolate the atomic install transaction
# from the real Swift build and code-signing tools.
APP_TEST="$WORK/app-install-failure"
APP_SHIMS="$APP_TEST/shims"
APP_BIN="$APP_TEST/swift-bin"
APP_HOME="$APP_TEST/home"
APP_CARGO_TARGET="$APP_TEST/cargo-target"
APP_BUILD_EVENTS="$APP_TEST/build-events.log"
TEST_VERSION=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$ROOT/AppBundle/Info.plist")
mkdir -p "$APP_SHIMS" "$APP_BIN/CowchatMac_CowchatMac.bundle" \
	"$APP_HOME/Applications/Cowchat.app/Contents" \
	"$APP_CARGO_TARGET/aarch64-apple-darwin/release" \
	"$APP_CARGO_TARGET/x86_64-apple-darwin/release"
printf '%s\n' 'previous-install' > "$APP_HOME/Applications/Cowchat.app/Contents/sentinel"
printf '%s\n' 'fake-universal-binary' > "$APP_BIN/CowchatMac"
for test_target in aarch64-apple-darwin x86_64-apple-darwin; do
	test_server="$APP_CARGO_TARGET/$test_target/release/cowchat-server"
	printf '%s\n' '#!/bin/sh' "printf '%s\\n' 'cowchat-server $TEST_VERSION'" > "$test_server"
	chmod +x "$test_server"
done

cat > "$APP_SHIMS/swift" <<'SHIM'
#!/bin/sh
case " $* " in
	*" --show-bin-path "*) printf '%s\n' "$COWCHAT_TEST_BIN_DIR" ;;
esac
SHIM
cat > "$APP_SHIMS/lipo" <<'SHIM'
#!/bin/sh
printf 'lipo:%s\n' "$*" >> "$COWCHAT_TEST_BUILD_EVENTS"
case " $* " in
	*" -create "*)
		output=''
		previous=''
		for argument do
			if [ "$previous" = "-output" ]; then output=$argument; break; fi
			previous=$argument
		done
		[ -n "$output" ] || exit 65
		cp "$1" "$output"
		chmod +x "$output"
		;;
esac
exit 0
SHIM
cat > "$APP_SHIMS/codesign" <<'SHIM'
#!/bin/sh
printf 'codesign:%s\n' "$*" >> "$COWCHAT_TEST_BUILD_EVENTS"
exit 0
SHIM
cat > "$APP_SHIMS/cargo" <<'SHIM'
#!/bin/sh
printf 'cargo:%s:%s\n' "${MACOSX_DEPLOYMENT_TARGET:-<missing>}" "$*" >> "$COWCHAT_TEST_BUILD_EVENTS"
exit 0
SHIM
cat > "$APP_SHIMS/rustup" <<'SHIM'
#!/bin/sh
case ${1:-} in
	target)
		printf '%s\n' 'aarch64-apple-darwin' 'x86_64-apple-darwin'
		;;
	which)
		for argument do tool_name=$argument; done
		tool_dir=$(CDPATH='' cd -- "$(dirname "$0")" && pwd)
		printf '%s/%s\n' "$tool_dir" "$tool_name"
		;;
	*) exit 64 ;;
esac
SHIM
cat > "$APP_SHIMS/rustc" <<'SHIM'
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
	"$APP_SHIMS/cargo" "$APP_SHIMS/rustc" "$APP_SHIMS/rustup" "$APP_SHIMS/xcrun" \
	"$APP_SHIMS/ditto" "$APP_SHIMS/mv"

if /usr/bin/env \
	HOME="$APP_HOME" \
	PATH="$APP_SHIMS:/usr/bin:/bin:/usr/sbin:/sbin" \
	CARGO_TARGET_DIR="$APP_CARGO_TARGET" \
	COWCHAT_TEST_BIN_DIR="$APP_BIN" \
	COWCHAT_TEST_BUILD_EVENTS="$APP_BUILD_EVENTS" \
	COWCHAT_TEST_FAIL_DITTO=1 \
	"$ROOT/build-app.sh" > "$APP_TEST/build.log" 2>&1; then
	printf '%s\n' 'The injected app copy failure unexpectedly succeeded.' >&2
	exit 1
fi
if ! grep -Fq 'cargo:13.0:build --locked --release --package cowchat-server --target aarch64-apple-darwin' "$APP_BUILD_EVENTS" || \
	! grep -Fq 'cargo:13.0:build --locked --release --package cowchat-server --target x86_64-apple-darwin' "$APP_BUILD_EVENTS"; then
	printf '%s\n' 'The app build did not request both locked server helper slices for macOS 13.' >&2
	exit 1
fi
dynamic_lipo_line=$(grep -n 'lipo:.* -create -output .*Contents/Helpers/cowchat-server' "$APP_BUILD_EVENTS" | head -n 1 | cut -d: -f1)
dynamic_server_sign_line=$(grep -n 'codesign:--force --sign - .*Contents/Helpers/cowchat-server' "$APP_BUILD_EVENTS" | head -n 1 | cut -d: -f1)
dynamic_app_sign_line=$(grep -n 'codesign:--force --sign - .*Cowchat.app$' "$APP_BUILD_EVENTS" | head -n 1 | cut -d: -f1)
if [ -z "$dynamic_lipo_line" ] || [ -z "$dynamic_server_sign_line" ] || [ -z "$dynamic_app_sign_line" ] || \
	[ "$dynamic_lipo_line" -ge "$dynamic_server_sign_line" ] || \
	[ "$dynamic_server_sign_line" -ge "$dynamic_app_sign_line" ]; then
	printf '%s\n' 'The app build did not assemble and sign the server helper inside-out.' >&2
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
	CARGO_TARGET_DIR="$APP_CARGO_TARGET" \
	COWCHAT_TEST_BIN_DIR="$APP_BIN" \
	COWCHAT_TEST_BUILD_EVENTS="$APP_BUILD_EVENTS" \
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
mkdir -p "$DMG_SHIMS" "$DMG_APP/Contents/MacOS" \
	"$DMG_APP/Contents/Helpers" "$DMG_TMP" "$DMG_OUTPUT"
cp "$ROOT/AppBundle/Info.plist" "$DMG_APP/Contents/Info.plist"
printf '%s\n' 'fake-universal-binary' > "$DMG_APP/Contents/MacOS/Cowchat"
printf '%s\n' '#!/bin/sh' "printf '%s\\n' 'cowchat-server $TEST_VERSION'" \
	> "$DMG_APP/Contents/Helpers/cowchat-server"
chmod +x "$DMG_APP/Contents/Helpers/cowchat-server"

cat > "$DMG_SHIMS/codesign" <<'SHIM'
#!/bin/sh
case " $* " in
	*" -dv "*)
		if [ "${COWCHAT_TEST_DEVELOPER_ID:-0}" = "1" ]; then
			for signed_path do :; done
			app_team=${COWCHAT_TEST_APP_TEAM_ID:-COWBOY123}
			case $signed_path in
				*/Contents/Helpers/cowchat-server)
					signing_team=${COWCHAT_TEST_HELPER_TEAM_ID:-$app_team}
					if [ "${COWCHAT_TEST_BAD_HELPER_AUTHORITY:-0}" = "1" ]; then
						authority="Authority=Developer ID Application: Other Corp ($signing_team)"
					else
						authority="Authority=Developer ID Application: Cowboy Inc. ($signing_team)"
					fi
					;;
				*)
					signing_team=$app_team
					authority="Authority=Developer ID Application: Cowboy Inc. ($signing_team)"
					;;
			esac
			printf '%s\n' "$authority" "TeamIdentifier=$signing_team" >&2
		else
			printf '%s\n' 'Signature=adhoc' >&2
		fi
		;;
	*" --verify --strict "*"/Contents/Helpers/cowchat-server "*)
		if [ "${COWCHAT_TEST_BAD_HELPER_SIGNATURE:-0}" = "1" ]; then exit 78; fi
		;;
esac
exit 0
SHIM
cat > "$DMG_SHIMS/lipo" <<'SHIM'
#!/bin/sh
case " $* " in
	*"/Contents/Helpers/cowchat-server -verify_arch arm64 x86_64 "*)
		if [ "${COWCHAT_TEST_BAD_HELPER_ARCH:-0}" = "1" ]; then exit 79; fi
		;;
esac
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

# Reject invalid bundled servers before creating or attaching an image, while
# preserving the last known-good DMG. The same verifier runs for the source,
# writable mount, and read-only mount.
BAD_HELPER_APP="$DMG_TEST/BadHelper.app"
cp -R "$DMG_APP" "$BAD_HELPER_APP"
printf '%s\n' '#!/bin/sh' "printf '%s\\n' 'cowchat-server 9.9.9'" \
	> "$BAD_HELPER_APP/Contents/Helpers/cowchat-server"
chmod +x "$BAD_HELPER_APP/Contents/Helpers/cowchat-server"
NONEXEC_HELPER_APP="$DMG_TEST/NonExecutableHelper.app"
cp -R "$DMG_APP" "$NONEXEC_HELPER_APP"
chmod 644 "$NONEXEC_HELPER_APP/Contents/Helpers/cowchat-server"

run_rejected_source_helper_test() {
	test_name=$1
	test_app=$2
	expected_error=$3
	bad_arch=$4
	bad_signature=$5
	developer_id=${6:-0}
	helper_team=${7:-COWBOY123}
	bad_helper_authority=${8:-0}
	expected_team=${9:-}
	log_file="$DMG_TEST/$test_name.log"
	printf '%s\n' 'previous-image' > "$DMG_OUTPUT/Cowchat-$TEST_VERSION.dmg"
	if /usr/bin/env \
		PATH="$DMG_SHIMS:/usr/bin:/bin:/usr/sbin:/sbin" \
		COWCHAT_DMG_OUTPUT_DIR="$DMG_OUTPUT" \
		COWCHAT_TEST_BAD_HELPER_ARCH="$bad_arch" \
		COWCHAT_TEST_BAD_HELPER_SIGNATURE="$bad_signature" \
		COWCHAT_TEST_DEVELOPER_ID="$developer_id" \
		COWCHAT_TEST_HELPER_TEAM_ID="$helper_team" \
		COWCHAT_TEST_BAD_HELPER_AUTHORITY="$bad_helper_authority" \
		COWCHAT_EXPECTED_TEAM_ID="$expected_team" \
		"$ROOT/build-dmg.sh" "$test_app" > "$log_file" 2>&1; then
		printf 'A DMG build with an invalid server helper unexpectedly succeeded (%s).\n' \
			"$test_name" >&2
		exit 1
	fi
	if ! grep -Fq "$expected_error" "$log_file"; then
		printf 'The DMG build did not report the invalid server helper (%s).\n' \
			"$test_name" >&2
		exit 1
	fi
	if [ "$(cat "$DMG_OUTPUT/Cowchat-$TEST_VERSION.dmg")" != "previous-image" ]; then
		printf 'A rejected server helper overwrote the previous DMG (%s).\n' \
			"$test_name" >&2
		exit 1
	fi
}

run_rejected_source_helper_test wrong-version "$BAD_HELPER_APP" \
	'Unexpected Cowchat server helper version (source app): cowchat-server 9.9.9' 0 0
run_rejected_source_helper_test non-executable "$NONEXEC_HELPER_APP" \
	'Cowchat server helper is missing or not executable (source app)' 0 0
run_rejected_source_helper_test wrong-architecture "$DMG_APP" \
	'Cowchat server helper is not universal (source app).' 1 0
run_rejected_source_helper_test invalid-signature "$DMG_APP" \
	'Cowchat server helper signature is invalid (source app)' 0 1
run_rejected_source_helper_test wrong-signing-team "$DMG_APP" \
	'Cowchat server helper TeamIdentifier does not match the app (source app): helper=OTHERTEAM app=COWBOY123' \
	0 0 1 OTHERTEAM 0 COWBOY123
run_rejected_source_helper_test wrong-signing-authority "$DMG_APP" \
	'Cowchat server helper signing authority does not match the app (source app).' \
	0 0 1 COWBOY123 1 COWBOY123

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
