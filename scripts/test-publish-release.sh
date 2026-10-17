#!/usr/bin/env bash
# The Python harness provides stateful CLI doubles; it never reaches the network.
set -euo pipefail
cd "$(dirname "$0")/.."
bash -n scripts/publish-release.sh
python3 scripts/test-publish-release.py
