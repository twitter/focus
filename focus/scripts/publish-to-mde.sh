#!/bin/sh
# Copyright 2022 Twitter, Inc.
# SPDX-License-Identifier: Apache-2.0


# Assumes ~/workspace/managed_development_environment exists.

set -euo pipefail


TMP_DIST_DIR=`mktemp -d`
FOCUS_DIST_DIR="$TMP_DIST_DIR/focus"
TARGET_TGZ="$TMP_DIST_DIR/focus.tar.gz"
ENTRYPOINT="focus"
REPO_ROOT="$(git rev-parse --show-toplevel)"
RELEASE_TARGET_ROOT="$(git rev-parse --show-toplevel)/target/release"

pushd $REPO_ROOT

env TARGET=x86_64-apple-darwin \
    MACOSX_DEPLOYMENT_TARGET=10.16 \
    RUSTFLAGS="-Awarnings" \
    cargo build --bin $ENTRYPOINT --release

mkdir -p "$FOCUS_DIST_DIR"

if [ ! -f "$RELEASE_TARGET_ROOT/$ENTRYPOINT" ]; then
	echo "Could not find $ENTRYPOINT in '$REPO_ROOT'. Exiting..."
    exit 1
fi

echo "(Dist dir: $FOCUS_DIST_DIR)"
cp $RELEASE_TARGET_ROOT/$ENTRYPOINT $FOCUS_DIST_DIR

popd

set -x
tar -czf "$TARGET_TGZ" --cd "$TMP_DIST_DIR" --exclude ".git" focus

pushd ~/workspace/managed_development_environment

mde-admin edit-package --upload-file "$TARGET_TGZ" --platform MacOSX --channel development eng.team.ee.experimental.focus focus

git commit -m "Updating eng.team.ee.experimental.focus with latest package" package/eng/team/ee/experimental/focus/group.json

git diff --quiet --exit-code || { echo "Local unstaged changes exist in MDE repo. Please fix."; exit 1; }
git diff --quiet --cached --exit-code || { echo "Local uncommitted changes exist in MDE repo. Please fix."; exit 1; }

arc diff --verbatim

popd
