#!/bin/bash
set -euo pipefail

set -euo pipefail
IFS=$'\n\t'

die() { echo "fatal: $*" >&2; exit 1; }

export TEMP
TEMP="$(mktemp -d 'git-env.XXXXXXXX' 2>/dev/null || mktemp -d -t tempdir)" || die "failed to make tmpdir"
TEMP="$(cd "$TEMP" && pwd -P)"

echo "TEMPDIR: $TEMP" >&2

# avoid ~/.gitconfig polluting the tests?
export HOME="$TEMP"

declare -xr BARE_REPO="$TEMP/bare.git"
declare -xr PLAIN_REPO="$TEMP/a"
declare -xr CLONED_REPO="${TEMP}/b"

setup() {
  # shellcheck disable=SC1091
  . "./setup-git-env.bash"
  if [[ -z "$TEMP" ]]; then
    echo "wtf no tmpdir!"
    exit 123
  fi

  set -x

  git init --bare "$BARE_REPO"
  git init "$PLAIN_REPO"
  (
    cd "$PLAIN_REPO"
    echo 'file' > a
    git add a
    git commit -am 'initial commit'
    git remote add origin "file://$BARE_REPO"
    git push origin master
    git tag 'tag1'
    git tag 'tag2'
    git tag 'tag3'
    git push --tags origin
    git push "file://$BARE_REPO" HEAD:refs/heads/foo/a HEAD:refs/heads/foo/b \
      HEAD:refs/heads/foo/c HEAD:refs/heads/bar/a HEAD:refs/heads/bar/b \
      HEAD:refs/heads/thunk HEAD:refs/heads/thud
  )

  echo "set up BARE_REPO: $BARE_REPO and PLAIN_REPO: $PLAIN_REPO" >&2
}

test_cloning_with_env() {
  setup

  declare -a git_env
  git_env=(
    'GIT_CONFIG_KEY_0=transfer.hideRefs'
    'GIT_CONFIG_VALUE_0=refs/tags'
    'GIT_CONFIG_KEY_1=transfer.hideRefs'
    'GIT_CONFIG_VALUE_1=refs/heads/bar'
    'GIT_CONFIG_KEY_2=transfer.hideRefs'
    'GIT_CONFIG_VALUE_2=refs/heads/bar/*'
    'GIT_CONFIG_COUNT=3'
  )

  /usr/bin/env "${git_env[@]}" git config --list

  /usr/bin/env "${git_env[@]}" \
    git clone "file://$BARE_REPO" "$CLONED_REPO"

  git -C "$CLONED_REPO" rev-parse 'refs/remotes/origin/bar/a' -- && false
}

test_cloning_with_env

