#!/bin/bash
set -euo pipefail
die() { echo "fatal: $*" >&2; exit 1; }


LOGFILE="/tmp/twgit-backend.log"

log() {
  printf '%s: %s\n' "$(date '+%Y-%m-%dT%H:%M:%S.%3N%:z')" "$*" >> "$LOGFILE"
}

HAS_USER_RE='^/source.git/([^/]+)/(git-(?:(?:upload|receive)-pack|info/refs))$'

GIT_CONFIG_COUNT=0

declare -a EXTRA_ENV

declare -r TX_HIDE_REFS_KEY='transfer.hideRefs'

EXTRA_ENV=(
  "GIT_CONFIG_KEY_0=transfer.hideRefs"
  "GIT_CONFIG_VALUE_0=refs/heads/*"
)

add_config_kv() {
  if [[ $# -ne 2 ]]; then
    die "add_config_kv requires two arguments, got $*"
  fi
  local k v
  k="$1"
  v="$2"

  EXTRA_ENV+=(
    "GIT_CONFIG_KEY_${GIT_CONFIG_COUNT}=${k}"
    "GIT_CONFIG_VALUE_${GIT_CONFIG_COUNT}=${v}"
  )

  GIT_CONFIG_COUNT=$(( GIT_CONFIG_COUNT + 1 ))
}

add_config_kv "$TX_HIDE_REFS_KEY" "refs/heads/*"

if [[ "${PATH_INFO}" =~ $HAS_USER_RE ]]; then
  add_config_kv "$TX_HIDE_REFS_KEY" "!refs/heads/${BASH_REMATCH[1]}"
  add_config_kv "$TX_HIDE_REFS_KEY" "!refs/heads/${BASH_REMATCH[1]}/*"

  # rewrite the path so the CGI can find it
  EXTRA_ENV+=(
    "PATH_INFO=/source.git/${BASH_REMATCH[2]}"
    ""
  )

fi

EXTRA_ENV+=("GIT_CONFIG_COUNT=${GIT_CONFIG_COUNT}")

log "adding env vars"
for pair in "${EXTRA_ENV[@]}"; do
  log "$pair"
done

exec /usr/bin/env "${EXTRA_ENV[@]}" /usr/lib/git-core/git-http-backend "$@"
