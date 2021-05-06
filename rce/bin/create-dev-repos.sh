#!/bin/bash
set -euo pipefail
IFS=$'\n\t'

die() { echo "fatal: $*" >&2; exit 1; }

if [[ $# -lt 3 ]]; then
  die "Usage: $(basename "$@") num-devs /path/to/source.git '/path/to/dev-%02d.git'"
fi

NUM_DEVS="$1"
shift
SOURCE_REPO="$1"
shift
DEST_PRINTF="$1"
shift

if ! [[ "$NUM_DEVS" =~ [0-9]+ ]]; then
  die "first argument must be a number"
fi

if [[ ! -d "$SOURCE_REPO" ]]; then
  die "source repo path $SOURCE_REPO did not exist"
fi

for n in $(seq 0 "$NUM_DEVS"); do
  # shellcheck disable=SC2059
  dest="$(printf "$DEST_PRINTF" "$n")"
  mkdir -p "$dest"
  tar -C "$SOURCE_REPO" -cf- . | pv | tar -C "$dest" -xf-
done

