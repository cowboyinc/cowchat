#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
VERIFY="$REPO_ROOT/.github/verify-release-tap-branch.sh"
TEST_ROOT=$(mktemp -d)
trap 'rm -rf "$TEST_ROOT"' EXIT
TAP="$TEST_ROOT/tap"

git init -q --initial-branch=main "$TAP"
cd "$TAP"
git config user.name human
git config user.email human@example.com
mkdir -p Formula Casks
printf '%s\n' old > Formula/cowchat.rb
printf '%s\n' old > Casks/cowchat.rb
git add Formula/cowchat.rb Casks/cowchat.rb
git commit -qm base

git checkout -qb bot-refresh
printf '%s\n' new > Formula/cowchat.rb
printf '%s\n' new > Casks/cowchat.rb
git add Formula/cowchat.rb Casks/cowchat.rb
git -c user.name=cowchat-release -c user.email=noreply@cowboy.inc commit -qm release
bash "$VERIFY" main HEAD

git checkout -qb human-edit
printf '%s\n' reviewed >> Formula/cowchat.rb
git add Formula/cowchat.rb
git commit -qm review
if bash "$VERIFY" main HEAD >/dev/null 2>&1; then
  echo "guard accepted a human-authored commit" >&2
  exit 1
fi

git checkout -q main
git checkout -qb human-committer
printf '%s\n' new > Formula/cowchat.rb
printf '%s\n' new > Casks/cowchat.rb
git add Formula/cowchat.rb Casks/cowchat.rb
git commit -qm release --author='cowchat-release <noreply@cowboy.inc>'
if bash "$VERIFY" main HEAD >/dev/null 2>&1; then
  echo "guard accepted a human-committed bot-authored commit" >&2
  exit 1
fi

git checkout -q main
git checkout -qb unexpected-path
printf '%s\n' new > Formula/cowchat.rb
printf '%s\n' new > Casks/cowchat.rb
printf '%s\n' unexpected > README.md
git add Formula/cowchat.rb Casks/cowchat.rb README.md
git -c user.name=cowchat-release -c user.email=noreply@cowboy.inc commit -qm release
if bash "$VERIFY" main HEAD >/dev/null 2>&1; then
  echo "guard accepted an unexpected changed path" >&2
  exit 1
fi

git checkout -q main
git checkout -qb incomplete-pair
printf '%s\n' new > Formula/cowchat.rb
git add Formula/cowchat.rb
git -c user.name=cowchat-release -c user.email=noreply@cowboy.inc commit -qm release
if bash "$VERIFY" main HEAD >/dev/null 2>&1; then
  echo "guard accepted an incomplete generated file pair" >&2
  exit 1
fi

git checkout -q main
git checkout -qb empty-branch
if bash "$VERIFY" main HEAD >/dev/null 2>&1; then
  echo "guard accepted a branch without a release-bot commit" >&2
  exit 1
fi

echo "release tap branch guard tests passed"
