#!/bin/bash

# this stuff has to go up here because git uses undefined vars in git-sh-setup
die() { echo "fatal: $*" >&2; exit 1; }

# shellcheck disable=SC1090
. "$(git --exec-path)/git-sh-setup" || die "couldn't source git-sh-setup"

set -euo pipefail
IFS=$'\n\t'

git checkout -b other >/dev/null

echo "abc" > abc
echo "def" > def
git add abc def >/dev/null
git commit -a -m 'add abc def' >/dev/null

show_ref="$(git show-ref -- "$(git symbolic-ref HEAD)")"
echo "${show_ref}"

