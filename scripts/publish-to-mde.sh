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

  beta)
    ;;

  development)
    ;;

  *)
    bail "Unknown channel \"$channel\""
    ;;
esac

set -o errexit

set -x

pushd $focus_repo
focus_rev=$(git rev-parse HEAD)
os=$(uname -s)
arch=$(uname -m)
target_tarball=$(mktemp -t "focus.$focus_rev.$os.$arch.tgz")

scripts/mac-build.sh || die "Build failed"
test -f target/release/$focus_binary || die "The build succeeded, but produced no binary"
echo $focus_rev > target/release/FOCUS_VERSION
tar czf $target_tarball -C target/release $focus_binary FOCUS_VERSION || die "Creating release tarball $target_tarball failed"

pushd $mde_repo
mde-admin edit-package --upload-file "$release_tarball" --platform MacOSX --channel $channel eng.team.ee.experimental.focus focus || die "Editing package with MDE failed"
popd # $mde_repo

popd # $focus_repo

rm $target_tarball
