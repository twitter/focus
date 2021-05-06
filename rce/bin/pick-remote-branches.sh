#!/bin/bash
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "Usage: $(dirname "$0") remote-url" >&2
  exit 1
fi

REMOTE_URL="$1"
shift

printf "\n  mark/unmark multiple branches to restore with 'TAB'/'S-TAB'\n\n" >&2

while read -r ref; do
  printf '%s:%s\n' "$ref" "refs/remotes/dev/${ref##refs/heads/}"
done < <(git ls-remote "$REMOTE_URL" | cut -f2 | fzf --multi)
