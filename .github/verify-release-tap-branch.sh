#!/usr/bin/env bash
set -euo pipefail

BASE_REF=${1:-origin/main}
HEAD_REF=${2:-HEAD}
BOT_NAME=cowchat-release
BOT_EMAIL=noreply@cowboy.inc

COMMITS=$(git rev-list "${BASE_REF}..${HEAD_REF}")
if [ -z "$COMMITS" ]; then
  echo "existing release branch has no release-bot commit" >&2
  exit 1
fi

for COMMIT in $COMMITS; do
  AUTHOR_NAME=$(git show -s --format=%an "$COMMIT")
  AUTHOR_EMAIL=$(git show -s --format=%ae "$COMMIT")
  COMMITTER_NAME=$(git show -s --format=%cn "$COMMIT")
  COMMITTER_EMAIL=$(git show -s --format=%ce "$COMMIT")
  if [ "$AUTHOR_NAME" != "$BOT_NAME" ] || [ "$AUTHOR_EMAIL" != "$BOT_EMAIL" ] \
    || [ "$COMMITTER_NAME" != "$BOT_NAME" ] || [ "$COMMITTER_EMAIL" != "$BOT_EMAIL" ]; then
    echo "existing release branch has a non-bot commit: $COMMIT" >&2
    echo "author: $AUTHOR_NAME <$AUTHOR_EMAIL>" >&2
    echo "committer: $COMMITTER_NAME <$COMMITTER_EMAIL>" >&2
    exit 1
  fi
done

EXPECTED_PATHS=$(printf '%s\n' Casks/cowchat.rb Formula/cowchat.rb)
ACTUAL_PATHS=$(git diff --name-only "${BASE_REF}...${HEAD_REF}" | LC_ALL=C sort)
if [ "$ACTUAL_PATHS" != "$EXPECTED_PATHS" ]; then
  echo "existing release branch changes files outside the generated formula/cask pair" >&2
  echo "expected:" >&2
  printf '%s\n' "$EXPECTED_PATHS" >&2
  echo "actual:" >&2
  printf '%s\n' "$ACTUAL_PATHS" >&2
  exit 1
fi
