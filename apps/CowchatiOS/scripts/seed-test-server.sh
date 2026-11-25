#!/bin/bash
# Makes the rooms live-test.sh expects on a fresh throwaway Cowchat server.
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: seed-test-server.sh <path-to-cowchat-cli> <api-key-file> [tcp-port]" >&2
  exit 64
fi
cli=$1
key=$(cat "$2")
port=${3:-9339}
peer() { "$cli" --tcp "127.0.0.1:$port" --key "$key" --name pinger --agent-id pinger-1 "$@"; }

peer rooms create design-review >/dev/null
peer send design-review "welcome to design review" >/dev/null
peer rooms create sealed --encrypted >/dev/null
peer --room-key 'correct horse battery' send sealed "the sealed plan is ready" >/dev/null
echo "seeded: design-review, sealed"
