#!/bin/bash
# Copyright 2022 Twitter, Inc.
# SPDX-License-Identifier: Apache-2.0


# This script prepares a copy of source acquired through 'source-init'
# to be a bare repo suitable for development with.
# This script should be run in a directory that is a copy of the source/.git
# directory. To set that up, run:
#
# $ rsync -avP source/.git/ baresource.git
#
# then cd into baresource.git and run this script.

die() { echo "fatal: $*" >&2; exit 1; }

# make it so git-sh-setup doesn't puke if we're not in a git directory
# so we can print a usage message
# shellcheck disable=SC2034
NONGIT_OK=1

# shellcheck disable=SC1090
source "$(git --exec-path)/git-sh-setup" || die "couldn't source git-sh-setup"

if ! git rev-parse --git-dir &>/dev/null; then
  die "you need to run this command in a git directory you wish to convert to a bare repo"
fi

git_dir_init

cd "$GIT_DIR" || die "could not cd to $GIT_DIR"

set -euo pipefail

if ! is_bare_repository &>/dev/null; then
  die "you need to run this script in a bare repo, i.e. a source/.git dir you copied"
fi

quietly_remove_section() {
  git config --remove-section --local "$@" &>/dev/null || true
}

has_key() {
  git config --local --get "$1" "$2" &>/dev/null
}

set_to_false() {
  if has_key "$1" 'true'; then
    git config --replace-all --local --bool "$1" 'false' 'true' || true
  fi
}

set_to_true() {
  if has_key "$1" 'false'; then
    git config --replace-all --local --bool "$1" 'true' 'false' || true
  fi
}

set -x

quietly_remove_section gc
quietly_remove_section 'completion.bash'
quietly_remove_section oh-my-zsh
quietly_remove_section index
quietly_remove_section indexhelper
quietly_remove_section sparsity
quietly_remove_section 'ci.alt'

set_to_false 'twitter.tricklefetch'
set_to_false 'twitter.statsenabled'
set_to_false 'twitter.usemultihooks'
set_to_false 'manageconfig.enable'
set_to_true 'core.bare'

quietly_remove_section 'remote.origin'

git config --local --add 'remote.origin.fetch' '+refs/heads/*:refs/heads/*'
git config --local --add 'remote.origin.url' 'https://git.twitter.biz/ro/source'

UNSET_KEYS=(
  core.watchmanignore
  core.preloadindex
  core.usewatchman
  core.fsmonitor
  core.fsmonitorhookversion
  core.alwaysontracing
  core.untrackedcache
  core.fastuntrackedfiles
  core.fsmonitorhookversion
  twitter.bulkfetch
  twitter.onetimeidlereapconfigchange
  twitter.onetimetricklefetchkill
  twitter.preflightcheck.newworkdir
)

for key in "${UNSET_KEYS[@]}"; do
  git config --local --unset-all "$key" || true
done

if [[ -f 'index' ]]; then
  rm -f 'index' || true
fi

readonly BOGUS_HEAD=refs/heads/BOGUSFORBAREREPO

if [[ "$(git symbolic-ref HEAD)" != "$BOGUS_HEAD" ]]; then
  git update-ref "$BOGUS_HEAD" "$(git rev-parse refs/heads/master)"
  git symbolic-ref HEAD "$BOGUS_HEAD"
fi

# go into detached HEAD mode so we can delete refs/heads/master
git update-ref -d refs/heads/master &>/dev/null || true

git pack-refs --all

sed -i'.bare-prep' -E -e 's,([0-9a-f]{40}) refs/remotes/origin/(.*),\1 refs/heads/\2,' packed-refs

git symbolic-ref HEAD refs/heads/master
git update-ref -d "$BOGUS_HEAD"

rm -rf objects/journals
mv pruned-odb/objects/pack/pack-*.{idx,pack} objects/pack/
rm -rf pruned-odb
rm -f objects/info/alternates

