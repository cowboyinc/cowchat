#!/usr/bin/env bash
set -euo pipefail

cowchat_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
repository_root="$(git -C "$cowchat_root" rev-parse --show-toplevel)"
contract_dir="$(mktemp -d "${TMPDIR:-/tmp}/cowchat-social-contract.XXXXXX")"
trap 'rm -f "$contract_dir/callback.json"; rmdir "$contract_dir"' EXIT
export COWCHAT_SOCIAL_CALLBACK_FIXTURE="$contract_dir/callback.json"

cd "$repository_root"
cargo test --locked -p cowchat-server --lib social::tests
test -s "$COWCHAT_SOCIAL_CALLBACK_FIXTURE"
swift test --package-path "$cowchat_root/apps/CowchatMac" --filter SocialTests/testServerCallbackContractFixtureAccepted
