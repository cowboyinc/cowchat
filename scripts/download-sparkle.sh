#!/bin/sh
set -eu
# Keep release tooling pinned separately from the SwiftPM framework checksum.
DEST=${1:?usage: download-sparkle.sh destination}
mkdir -p "$DEST"
ARCHIVE="$DEST/Sparkle-2.10.0.tar.xz"
curl --fail --location --retry 3 --output "$ARCHIVE" \
  https://github.com/sparkle-project/Sparkle/releases/download/2.10.0/Sparkle-2.10.0.tar.xz
printf '%s  %s\n' c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c "$ARCHIVE" | shasum -a 256 --check
mkdir -p "$DEST/tools"
tar -xJf "$ARCHIVE" -C "$DEST/tools"
