#!/bin/sh
# Source from build-app.sh or packaging verification. No Keychain access for '-'.
sign_sparkle_framework() {
    sparkle_framework=$1
    sparkle_identity=$2
    sparkle_version="$sparkle_framework/Versions/B"
    # Preserve the vendor's sandbox entitlements on the XPC services.
    # Sign each nested object before its containing bundle. Never deep-sign.
    for sparkle_component in XPCServices/Installer.xpc XPCServices/Downloader.xpc Autoupdate Updater.app; do
        if [ "$sparkle_identity" = "-" ]; then
            codesign --force --sign - --preserve-metadata=entitlements \
                "$sparkle_version/$sparkle_component" >/dev/null
        else
            codesign --force --options runtime --timestamp --sign "$sparkle_identity" \
                --preserve-metadata=entitlements "$sparkle_version/$sparkle_component" >/dev/null
        fi
    done
    if [ "$sparkle_identity" = "-" ]; then
        codesign --force --sign - "$sparkle_framework" >/dev/null
    else
        codesign --force --options runtime --timestamp --sign "$sparkle_identity" \
            "$sparkle_framework" >/dev/null
    fi
    codesign --verify --deep --strict "$sparkle_framework"
}
