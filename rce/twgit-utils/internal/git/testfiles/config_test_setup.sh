#!/bin/bash

# this stuff has to go up here because git uses undefined vars in git-sh-setup
die() { echo "fatal: $*" >&2; exit 1; }

# shellcheck disable=SC1090
. "$(git --exec-path)/git-sh-setup" || die "couldn't source git-sh-setup"

set -euo pipefail

exec 3>&1 # save stdout for later
exec 1>&2 # all these git commands will have stdout redir to stderr

git config --global "global.one.a" "a"
git config --global "global.one.b" "b"
git config --global "global.one.c" "c"

git config --local "test.one.a" "a"
git config --local "test.one.b" "b"
git config --local "test.one.c" "c"

git config --local --type=int "test.int.a" "1"

echo "SUCCESS" >&3  # say this on the original stdout
