#!/bin/bash

function die() {
    echo "FATAL: $@" >/dev/stderr
    exit 1
}

focus_repo=$(git rev-parse --show-toplevel)
mde_repo="$HOME/workspace/managed_development_environment"
focus_binary="focus"

channel=$1
shift

case $channel in

  release)
    ;;

  development)
    ;;

  *)
    die "Unknown channel \"$channel\""
    ;;
esac

set -x

pushd $focus_repo
focus_rev=$(git rev-parse HEAD)
os=$(uname -s)
arch=$(uname -m)
target_tarball=$(mktemp -t "focus.$focus_rev.$os.$arch.tgz")

scripts/mac-build.sh || die "Build failed"
test -f target/release/$focus_binary || die "The build succeeded, but produced no binary"
rm -rf target/package
mkdir -p target/package/focus/bin
echo $focus_rev > target/package/focus/bin/FOCUS_VERSION
cp target/release/$focus_binary target/package/focus/bin/focus
tar czf $target_tarball -C target/package focus || die "Creating release tarball $target_tarball failed"

pushd $mde_repo

# git reset --hard || die "git reset in $mde_repo failed"
# git pull --quiet || die "git pull in $mde_repo failed"
mde-admin edit-package --upload-file $target_tarball --git-sha $focus_rev --platform MacOSX --channel $channel eng.team.ee.experimental.focus focus || die "Editing package with MDE failed"
git add package/eng/team/ee/experimental/focus
git commit -am "Update focus $focus_rev $channel channel AUTOMATED_COMMIT=true"
popd # $mde_repo

popd # $focus_repo

rm $target_tarball
