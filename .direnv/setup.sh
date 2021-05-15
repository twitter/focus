#!/bin/bash
set -euo pipefail
IFS=$'\n\t'

die() { echo "fatal: $*" >&2; exit 1; }

TLD="$(git rev-parse --show-toplevel)" || die "could not determine toplevel"
cd "$TLD"  || die "failed to cd to $TLD"

rm -f .envrc
ln -s .direnv/envrc .envrc
rm -f rce/repo_tools/.envrc
ln -s ../../.direnv/rce/repo_tools/envrc rce/repo_tools/.envrc
rm -rf rce/twgit-utils/.envrc
ln -s ../../.direnv/rce/twgit-utils/envrc rce/twgit-utils/.envrc
