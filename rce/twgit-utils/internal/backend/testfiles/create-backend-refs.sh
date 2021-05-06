#!/bin/bash

set -euo pipefail
IFS=$'\n\t'

die() { echo "fatal: $*" >&2; exit 1; }

if [[ -z "$TEST_REPO" ]]; then
  die "you must set TEST_REPO in the environment"
fi

cd "$TEST_REPO"

