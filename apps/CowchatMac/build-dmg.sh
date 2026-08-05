#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname "$0")" && pwd)
BACKGROUND_SVG="$ROOT/DmgBackground.svg"
OUTPUT_DIR=${COWCHAT_DMG_OUTPUT_DIR:-"$ROOT/dist"}
SIGNING_IDENTITY=${COWCHAT_CODESIGN_IDENTITY:-}
NOTARY_PROFILE=${COWCHAT_NOTARY_PROFILE:-}
EXPECTED_TEAM_ID=${COWCHAT_EXPECTED_TEAM_ID:-}
VOLUME_NAME="Cowchat"
WINDOW_WIDTH=660
WINDOW_HEIGHT=450
APP_POSITION_X=180
APP_POSITION_Y=245
APPLICATIONS_POSITION_X=480
APPLICATIONS_POSITION_Y=245

usage() {
	cat <<'EOF'
Usage: ./build-dmg.sh [path/to/Cowchat.app]

Packages an existing, signed Cowchat.app in a Finder-configured drag-to-install
disk image. The default app is ~/Applications/Cowchat.app. Override the output
directory with COWCHAT_DMG_OUTPUT_DIR.

Set COWCHAT_CODESIGN_IDENTITY to sign the DMG with the same Developer ID used
for the app. Set COWCHAT_NOTARY_PROFILE to notarize, staple, and Gatekeeper-check
the result through a notarytool keychain profile. Without both, the result is a
local/development artifact rather than a downloadable release installer.

This build needs a logged-in Finder session. It fails if Finder cannot write and
then verify the icon positions, background, and window metadata.
EOF
}

case ${1:-} in
	-h|--help)
		usage
		exit 0
		;;
esac
if [ "$#" -gt 1 ]; then
	usage >&2
	exit 64
fi

SOURCE_APP=${1:-${COWCHAT_APP_PATH:-"$HOME/Applications/Cowchat.app"}}
if [ ! -d "$SOURCE_APP" ]; then
	printf 'Missing Cowchat app: %s\n' "$SOURCE_APP" >&2
	printf '%s\n' 'Run ./build-app.sh first or pass an existing Cowchat.app path.' >&2
	exit 1
fi
if [ ! -f "$BACKGROUND_SVG" ]; then
	printf 'Missing DMG background: %s\n' "$BACKGROUND_SVG" >&2
	exit 1
fi

for tool in codesign ditto diskutil find hdiutil lipo osascript plutil readlink sips xmllint; do
	if ! command -v "$tool" >/dev/null 2>&1; then
		printf 'Missing required macOS tool: %s\n' "$tool" >&2
		exit 1
	fi
done

SOURCE_APP_PARENT=$(CDPATH='' cd -- "$(dirname "$SOURCE_APP")" && pwd)
SOURCE_APP="$SOURCE_APP_PARENT/$(basename "$SOURCE_APP")"
INFO_PLIST="$SOURCE_APP/Contents/Info.plist"
if [ ! -f "$INFO_PLIST" ]; then
	printf 'Missing app Info.plist: %s\n' "$INFO_PLIST" >&2
	exit 1
fi

VERSION=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$INFO_PLIST" 2>/dev/null || true)
BUNDLE_ID=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$INFO_PLIST" 2>/dev/null || true)
if [ "$BUNDLE_ID" != "inc.cowboy.cowchat" ]; then
	printf 'Unexpected bundle identifier: %s\n' "${BUNDLE_ID:-<missing>}" >&2
	exit 1
fi
case $VERSION in
	''|*[!A-Za-z0-9._-]*)
		printf 'Unsafe or missing app version: %s\n' "${VERSION:-<missing>}" >&2
		exit 1
		;;
esac

if ! codesign --verify --deep --strict "$SOURCE_APP"; then
	printf 'Cowchat.app is not validly signed: %s\n' "$SOURCE_APP" >&2
	exit 1
fi
if ! lipo "$SOURCE_APP/Contents/MacOS/Cowchat" -verify_arch arm64 x86_64; then
	printf '%s\n' 'Cowchat.app is not universal (arm64 + x86_64).' >&2
	printf '%s\n' 'Rebuild it with the current ./build-app.sh before packaging.' >&2
	exit 1
fi

APP_SIGNATURE_INFO=$(codesign -dv --verbose=4 "$SOURCE_APP" 2>&1 || true)
APP_IS_DEVELOPER_ID=0
if printf '%s\n' "$APP_SIGNATURE_INFO" | /usr/bin/grep -Fq 'Authority=Developer ID Application:'; then
	APP_IS_DEVELOPER_ID=1
fi
if printf '%s\n' "$APP_SIGNATURE_INFO" | /usr/bin/grep -Fq 'Signature=adhoc'; then
	if [ -n "$SIGNING_IDENTITY" ] || [ -n "$NOTARY_PROFILE" ]; then
		printf '%s\n' 'Cowchat.app is ad-hoc signed; rebuild it with COWCHAT_CODESIGN_IDENTITY first.' >&2
		exit 1
	fi
	printf '%s\n' 'Warning: packaging an ad-hoc signed app for local/development use only.' >&2
fi
if [ -n "$EXPECTED_TEAM_ID" ] && \
	! printf '%s\n' "$APP_SIGNATURE_INFO" | /usr/bin/grep -Fq "TeamIdentifier=$EXPECTED_TEAM_ID"; then
	printf 'Cowchat.app is not signed by expected team %s.\n' "$EXPECTED_TEAM_ID" >&2
	exit 1
fi
if [ -n "$NOTARY_PROFILE" ] && [ -z "$SIGNING_IDENTITY" ]; then
	printf '%s\n' 'COWCHAT_NOTARY_PROFILE requires COWCHAT_CODESIGN_IDENTITY for the DMG.' >&2
	exit 1
fi
if [ -n "$NOTARY_PROFILE" ] && [ "$APP_IS_DEVELOPER_ID" -ne 1 ]; then
	printf '%s\n' 'Notarization requires Cowchat.app to have a Developer ID Application signature.' >&2
	exit 1
fi

if [ -e "/Volumes/$VOLUME_NAME" ]; then
	printf 'A volume named %s is already mounted; detach it before building.\n' "$VOLUME_NAME" >&2
	exit 1
fi

if ! osascript -e 'with timeout of 30 seconds' \
	-e 'tell application "Finder" to get name of startup disk' \
	-e 'end timeout' >/dev/null; then
	printf '%s\n' 'Finder automation is unavailable.' >&2
	printf '%s\n' 'Unlock the Mac and allow the calling terminal application to automate Finder.' >&2
	exit 1
fi

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR=$(CDPATH='' cd -- "$OUTPUT_DIR" && pwd)
TMP_BASE=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
WORK=$(mktemp -d "$TMP_BASE/cowchat-dmg.XXXXXX")
STAGE_DIR="$WORK/stage"
RW_DMG="$WORK/Cowchat-rw.dmg"
FINAL_STEM="$WORK/Cowchat-final"
FINAL_DMG="$FINAL_STEM.dmg"
ATTACHED_DEVICE=''
MOUNT_POINT=''
ATTACHED_IMAGE=''
ATTACH_MAY_BE_MOUNTED=0
PUBLISH_DIR=''

resolve_attachment_from_system() {
	[ -n "$ATTACHED_IMAGE" ] || return 1
	info_plist="$WORK/hdiutil-info.plist"
	if ! hdiutil info -plist > "$info_plist"; then
		return 1
	fi
	image_index=0
	while candidate_image=$(/usr/libexec/PlistBuddy \
		-c "Print :images:$image_index:image-path" "$info_plist" 2>/dev/null); do
		if [ "$candidate_image" = "$ATTACHED_IMAGE" ]; then
			entity_index=0
			fallback_device=''
			while candidate_device=$(/usr/libexec/PlistBuddy \
				-c "Print :images:$image_index:system-entities:$entity_index:dev-entry" \
				"$info_plist" 2>/dev/null); do
				if [ -z "$fallback_device" ]; then fallback_device=$candidate_device; fi
				candidate_mount=$(/usr/libexec/PlistBuddy \
					-c "Print :images:$image_index:system-entities:$entity_index:mount-point" \
					"$info_plist" 2>/dev/null || true)
				if [ -n "$candidate_mount" ]; then
					ATTACHED_DEVICE=$candidate_device
					MOUNT_POINT=$candidate_mount
					return 0
				fi
				entity_index=$((entity_index + 1))
			done
			if [ -n "$fallback_device" ]; then
				ATTACHED_DEVICE=$fallback_device
				return 0
			fi
			return 1
		fi
		image_index=$((image_index + 1))
	done
	return 1
}

cleanup() {
	cleanup_status=$?
	trap - EXIT HUP INT TERM
	cleanup_failed=0
	if [ "$ATTACH_MAY_BE_MOUNTED" -eq 1 ] && \
		[ -z "$ATTACHED_DEVICE" ] && [ -z "$MOUNT_POINT" ]; then
		resolve_attachment_from_system || true
	fi
	detach_target=$ATTACHED_DEVICE
	if [ -z "$detach_target" ]; then
		detach_target=$MOUNT_POINT
	fi
	if [ "$ATTACH_MAY_BE_MOUNTED" -eq 1 ]; then
		if [ -n "$detach_target" ] && {
			hdiutil detach "$detach_target" >/dev/null 2>&1 ||
				hdiutil detach -force "$detach_target" >/dev/null 2>&1;
		}; then
			ATTACH_MAY_BE_MOUNTED=0
		else
			cleanup_failed=1
			printf '%s\n' 'Could not detach the temporary Cowchat disk image.' >&2
			printf 'Preserving recovery files at: %s\n' "$WORK" >&2
		fi
	fi
	if [ -n "$PUBLISH_DIR" ]; then
		case $PUBLISH_DIR in
			"$OUTPUT_DIR"/.cowchat-publish.*)
				if ! /bin/rm -rf -- "$PUBLISH_DIR"; then
					cleanup_failed=1
				fi
				;;
			*)
				printf 'Refusing to clean unexpected publish path: %s\n' "$PUBLISH_DIR" >&2
				cleanup_failed=1
				;;
		esac
	fi
	if [ "$cleanup_failed" -eq 0 ]; then
		case $WORK in
			"$TMP_BASE"/cowchat-dmg.*)
				if ! /bin/rm -rf -- "$WORK"; then
					cleanup_failed=1
				fi
				;;
			*)
				printf 'Refusing to clean unexpected temporary path: %s\n' "$WORK" >&2
				cleanup_failed=1
				;;
		esac
	fi
	if [ "$cleanup_status" -eq 0 ] && [ "$cleanup_failed" -ne 0 ]; then
		cleanup_status=1
	fi
	exit "$cleanup_status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

attach_image() {
	image_path=$1
	attach_mode=$2
	attach_plist="$WORK/attach.plist"

	ATTACHED_IMAGE=$image_path
	ATTACH_MAY_BE_MOUNTED=1
	if hdiutil attach "$image_path" "$attach_mode" -noverify -noautoopen -plist > "$attach_plist"; then
		:
	else
		attach_status=$?
		resolve_attachment_from_system || true
		return "$attach_status"
	fi
	# Record the detachable device before validating the mount point. If plist
	# parsing fails after attach, cleanup can still detach or preserve the image.
	ATTACHED_DEVICE=$(xmllint --xpath 'string(//dict[key="mount-point"][1]/key[.="dev-entry"]/following-sibling::string[1])' "$attach_plist" 2>/dev/null || true)
	MOUNT_POINT=$(xmllint --xpath 'string(//key[.="mount-point"][1]/following-sibling::string[1])' "$attach_plist" 2>/dev/null || true)
	if [ -z "$MOUNT_POINT" ] || [ -z "$ATTACHED_DEVICE" ] || [ ! -d "$MOUNT_POINT" ]; then
		printf '%s\n' 'Could not resolve the attached DMG device and mount point.' >&2
		exit 1
	fi
	if [ "$MOUNT_POINT" != "/Volumes/$VOLUME_NAME" ]; then
		printf 'Unexpected mount point: %s\n' "$MOUNT_POINT" >&2
		exit 1
	fi
}

detach_image() {
	if [ -z "$ATTACHED_DEVICE" ]; then
		return
	fi
	hdiutil detach "$ATTACHED_DEVICE" >/dev/null
	ATTACHED_DEVICE=''
	MOUNT_POINT=''
	ATTACHED_IMAGE=''
	ATTACH_MAY_BE_MOUNTED=0
}

verify_payload() {
	volume_root=$1
	verification_phase=${2:-mounted}
	if [ ! -d "$volume_root/Cowchat.app" ]; then
		printf '%s\n' 'DMG is missing Cowchat.app.' >&2
		exit 1
	fi
	if [ ! -L "$volume_root/Applications" ] || [ "$(readlink "$volume_root/Applications")" != "/Applications" ]; then
		printf '%s\n' 'DMG Applications item is not a symlink to /Applications.' >&2
		exit 1
	fi
	if [ ! -f "$volume_root/.background/background.png" ]; then
		printf '%s\n' 'DMG is missing its Finder background.' >&2
		exit 1
	fi
	if [ ! -f "$volume_root/.DS_Store" ]; then
		printf '%s\n' 'DMG is missing Finder layout metadata.' >&2
		exit 1
	fi
	# Finder 26 can set the background but raises an AppleEvent error when its
	# background-picture property is read back. Verify the persisted DS_Store
	# records instead, on both the writable and final read-only mounts.
	if ! /usr/bin/strings "$volume_root/.DS_Store" | /usr/bin/grep -Fq 'backgroundImageAlias' || \
		! /usr/bin/strings "$volume_root/.DS_Store" | /usr/bin/grep -Fq 'background.png'; then
		printf 'DMG Finder metadata does not reference its background image (%s mount).\n' "$verification_phase" >&2
		exit 1
	fi
	visible_count=$(find "$volume_root" -mindepth 1 -maxdepth 1 ! -name '.*' | wc -l | tr -d ' ')
	if [ "$visible_count" != "2" ]; then
		printf 'DMG contains %s visible root items; expected Cowchat.app and Applications.\n' "$visible_count" >&2
		exit 1
	fi
	codesign --verify --deep --strict "$volume_root/Cowchat.app"
	if ! lipo "$volume_root/Cowchat.app/Contents/MacOS/Cowchat" -verify_arch arm64 x86_64; then
		printf 'Cowchat.app lost a supported architecture in the %s image.\n' "$verification_phase" >&2
		exit 1
	fi
}

configure_and_verify_finder() {
	mount_root=$1
	mode=$2

	if ! osascript - "$VOLUME_NAME" "$mode" "$WINDOW_WIDTH" "$WINDOW_HEIGHT" \
		"$APP_POSITION_X" "$APP_POSITION_Y" "$APPLICATIONS_POSITION_X" "$APPLICATIONS_POSITION_Y" <<'APPLESCRIPT'
on run argv
	set volumeName to item 1 of argv
	set operationMode to item 2 of argv
	set windowWidth to (item 3 of argv) as integer
	set windowHeight to (item 4 of argv) as integer
	set appX to (item 5 of argv) as integer
	set appY to (item 6 of argv) as integer
	set applicationsX to (item 7 of argv) as integer
	set applicationsY to (item 8 of argv) as integer
	set backgroundFile to POSIX file ("/Volumes/" & volumeName & "/.background/background.png") as alias

	with timeout of 30 seconds
		tell application "Finder"
			tell disk volumeName
				open
				delay 1
				set dmgWindow to container window
				tell dmgWindow
					if operationMode is "configure" or operationMode is "configure-retry" then
						set current view to icon view
						delay 1
						-- Keep an explicit window reference. On macOS 26, asking for
						-- "icon view options" through the implicit nested tell target
						-- produces an unusable specifier even though the window is valid.
						set viewOptions to icon view options of dmgWindow
						-- Do not read configured properties back before closing: Finder 26
						-- otherwise fails to flush the configured DS_Store records. A
						-- redundant arrangement assignment has the same Finder bug.
						if arrangement of viewOptions is not not arranged then
							set arrangement of viewOptions to not arranged
						end if
						set icon size of viewOptions to 112
						set text size of viewOptions to 13
						set label position of viewOptions to bottom
						set background picture of viewOptions to backgroundFile
						set position of item "Cowchat.app" to {appX, appY}
						set position of item "Applications" to {applicationsX, applicationsY}
						-- Finder 26 only flushes both record groups reliably when icon
						-- metadata is changed before window chrome and bounds.
						set toolbar visible to false
						set statusbar visible to false
						set pathbar visible to false
						set sidebar width to 0
						set bounds to {100, 100, 100 + windowWidth, 100 + windowHeight}
						delay 2
					else
						if current view is not icon view then error "DMG window is not in icon view"
						set viewOptions to icon view options of dmgWindow
						if arrangement of viewOptions is not not arranged then error "DMG icons are arranged automatically"
						if icon size of viewOptions is not 112 then error "Unexpected DMG icon size"
						if text size of viewOptions is not 13 then error "Unexpected DMG text size"
						if label position of viewOptions is not bottom then error "Unexpected DMG label position"
						if position of item "Cowchat.app" is not {appX, appY} then error "Cowchat.app is misplaced"
						if position of item "Applications" is not {applicationsX, applicationsY} then error "Applications is misplaced"
						if toolbar visible is not false then error "DMG toolbar is visible"
						if statusbar visible is not false then error "DMG status bar is visible"
						if pathbar visible is not false then error "DMG path bar is visible"
						if bounds is not {100, 100, 100 + windowWidth, 100 + windowHeight} then error "Unexpected DMG window bounds"
					end if
					close
				end tell
			end tell
		end tell
	end timeout
end run
APPLESCRIPT
	then
		printf '%s\n' 'Finder could not configure or verify the DMG layout.' >&2
		printf '%s\n' 'Run this build from a logged-in macOS desktop session with Finder available.' >&2
		exit 1
	fi

	if [ "$mode" = "configure" ] || [ "$mode" = "configure-retry" ]; then
		attempt=0
		while [ "$attempt" -lt 10 ]; do
			if [ -f "$mount_root/.DS_Store" ] && \
				/usr/bin/strings "$mount_root/.DS_Store" | /usr/bin/grep -Fq 'backgroundImageAlias' && \
				/usr/bin/strings "$mount_root/.DS_Store" | /usr/bin/grep -Fq 'background.png'; then
				break
			fi
			sleep 1
			attempt=$((attempt + 1))
		done
		if [ "$mode" = "configure" ] && { \
			[ ! -f "$mount_root/.DS_Store" ] || \
			! /usr/bin/strings "$mount_root/.DS_Store" | /usr/bin/grep -Fq 'backgroundImageAlias' || \
			! /usr/bin/strings "$mount_root/.DS_Store" | /usr/bin/grep -Fq 'background.png'; \
		}; then
			# Finder 26 occasionally creates its default DS_Store before the
			# background alias is ready, then refuses to amend that file. A
			# single clean retry on the same validated temporary volume is
			# deterministic once Finder has registered the mount.
			if [ -f "$mount_root/.DS_Store" ]; then
				unlink "$mount_root/.DS_Store"
			fi
			sync
			sleep 2
			configure_and_verify_finder "$mount_root" configure-retry
		fi
		sync
	fi
}

mkdir -p "$STAGE_DIR/.background"
ditto "$SOURCE_APP" "$STAGE_DIR/Cowchat.app"
ln -s /Applications "$STAGE_DIR/Applications"
touch "$STAGE_DIR/.metadata_never_index"
sips -s format png "$BACKGROUND_SVG" --out "$STAGE_DIR/.background/background.png" >/dev/null

BACKGROUND_WIDTH=$(sips -g pixelWidth "$STAGE_DIR/.background/background.png" | awk '/pixelWidth:/ { print $2 }')
BACKGROUND_HEIGHT=$(sips -g pixelHeight "$STAGE_DIR/.background/background.png" | awk '/pixelHeight:/ { print $2 }')
if [ "$BACKGROUND_WIDTH" != "660" ] || [ "$BACKGROUND_HEIGHT" != "420" ]; then
	printf 'Unexpected DMG background size: %sx%s\n' "$BACKGROUND_WIDTH" "$BACKGROUND_HEIGHT" >&2
	exit 1
fi

SIZE_KB=$(du -sk "$STAGE_DIR" | awk '{ print $1 }')
SIZE_MB=$((SIZE_KB / 1024 + 24))
hdiutil create -quiet -size "${SIZE_MB}m" -fs HFS+ -volname "$VOLUME_NAME" -srcfolder "$STAGE_DIR" -format UDRW "$RW_DMG"

attach_image "$RW_DMG" -readwrite
# Finder 26 can open a just-attached image before its hidden background file is
# available to the scripting layer, silently dropping the background alias.
sleep 2
configure_and_verify_finder "$MOUNT_POINT" configure
verify_payload "$MOUNT_POINT" writable
sync
detach_image

hdiutil convert -quiet "$RW_DMG" -format UDZO -imagekey zlib-level=9 -o "$FINAL_STEM"
if [ ! -f "$FINAL_DMG" ]; then
	printf 'DMG conversion did not produce: %s\n' "$FINAL_DMG" >&2
	exit 1
fi

attach_image "$FINAL_DMG" -readonly
DISK_INFO="$WORK/disk-info.plist"
diskutil info -plist "$MOUNT_POINT" > "$DISK_INFO"
WRITABLE_VOLUME=$(plutil -extract WritableVolume raw -o - "$DISK_INFO")
if [ "$WRITABLE_VOLUME" != "false" ]; then
	printf 'Final DMG did not mount read-only (WritableVolume=%s).\n' "$WRITABLE_VOLUME" >&2
	exit 1
fi
verify_payload "$MOUNT_POINT" read-only
configure_and_verify_finder "$MOUNT_POINT" verify
detach_image

OUTPUT_DMG="$OUTPUT_DIR/Cowchat-$VERSION.dmg"
PUBLISH_DIR=$(mktemp -d "$OUTPUT_DIR/.cowchat-publish.XXXXXX")
TEMP_OUTPUT="$PUBLISH_DIR/Cowchat-$VERSION.dmg"
ditto "$FINAL_DMG" "$TEMP_OUTPUT"

if [ -n "$SIGNING_IDENTITY" ]; then
	codesign --force --timestamp --sign "$SIGNING_IDENTITY" "$TEMP_OUTPUT" >/dev/null
	codesign --verify --verbose=2 "$TEMP_OUTPUT"
	DMG_SIGNATURE_INFO=$(codesign -dv --verbose=4 "$TEMP_OUTPUT" 2>&1 || true)
	if [ -n "$EXPECTED_TEAM_ID" ] && \
		! printf '%s\n' "$DMG_SIGNATURE_INFO" | /usr/bin/grep -Fq "TeamIdentifier=$EXPECTED_TEAM_ID"; then
		printf 'Cowchat DMG is not signed by expected team %s.\n' "$EXPECTED_TEAM_ID" >&2
		exit 1
	fi
	if [ -n "$NOTARY_PROFILE" ] && \
		! printf '%s\n' "$DMG_SIGNATURE_INFO" | /usr/bin/grep -Fq 'Authority=Developer ID Application:'; then
		printf '%s\n' 'Notarization requires a Developer ID Application signature on the DMG.' >&2
		exit 1
	fi
fi
hdiutil verify "$TEMP_OUTPUT" >/dev/null

if [ -n "$NOTARY_PROFILE" ]; then
	for tool in spctl xcrun; do
		if ! command -v "$tool" >/dev/null 2>&1; then
			printf 'Missing required notarization tool: %s\n' "$tool" >&2
			exit 1
		fi
	done
	xcrun notarytool submit "$TEMP_OUTPUT" --keychain-profile "$NOTARY_PROFILE" --wait
	xcrun stapler staple "$TEMP_OUTPUT"
	xcrun stapler validate "$TEMP_OUTPUT"
	spctl --assess --type open --context context:primary-signature --verbose=4 "$TEMP_OUTPUT"
elif [ -n "$SIGNING_IDENTITY" ]; then
	printf '%s\n' 'Warning: the DMG is signed but not notarized; it is not a finished downloadable release.' >&2
else
	printf '%s\n' 'Warning: the DMG is unsigned and intended only for local/development use.' >&2
fi
hdiutil verify "$TEMP_OUTPUT" >/dev/null

# Publish only after every validation has succeeded, so a failed build cannot
# overwrite the last known-good image.
mv -f "$TEMP_OUTPUT" "$OUTPUT_DMG"
rmdir "$PUBLISH_DIR"
PUBLISH_DIR=''

printf 'Built and verified: %s\n' "$OUTPUT_DMG"
