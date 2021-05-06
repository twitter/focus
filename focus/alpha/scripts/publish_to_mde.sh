#!/bin/sh

# Assumes ~/workspace/managed_development_environment exists.

set -euo pipefail

TMP_DIST_DIR=`mktemp -d`
FOCUS_DIST_DIR="$TMP_DIST_DIR/focus"
TARGET_TGZ="$TMP_DIST_DIR/focus.tar.gz"
ENTRYPOINT="focus-alpha"
ALPHA_ROOT="$(git rev-parse --show-toplevel)/alpha"

pushd $ALPHA_ROOT

mkdir -p "$FOCUS_DIST_DIR"

if [ ! -f "$ALPHA_ROOT/$ENTRYPOINT" ]; then
	echo "Could not find $ENTRYPOINT in '$ALPHA_ROOT'. Exiting..."
    exit 1
fi

cp -R "$ALPHA_ROOT/" $FOCUS_DIST_DIR

popd

tar -czf "$TARGET_TGZ" --cd "$TMP_DIST_DIR" --exclude ".git" focus

pushd ~/workspace/managed_development_environment

mde-admin edit-package --upload-file "$TARGET_TGZ" --platform MacOSX --channel development eng.team.ee.experimental.focus focus

git commit -m "Updating eng.team.ee.experimental.focus with latest package" package/eng/team/ee/experimental/focus/group.json

git diff --quiet --exit-code || { echo "Local unstaged changes exist in MDE repo. Please fix."; exit 1; }
git diff --quiet --cached --exit-code || { echo "Local uncommitted changes exist in MDE repo. Please fix."; exit 1; }

arc diff --verbatim

popd

