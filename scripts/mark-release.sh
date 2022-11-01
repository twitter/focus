#!/bin/bash
# Copyright 2022 Twitter, Inc.
# SPDX-License-Identifier: Apache-2.0


# This is a script to change the versions of focus and push up a branch and a tag.
# Afterwards, you may have some changes to your Cargo.lock file. Blame some background process like rust-analyzer in vscode

set -xev
if [ -z "$1" ]; then
    echo "Usage: mark-release.sh NEW_VESRION"
    exit 1
fi

if [[ $(git diff --stat) != '' ]]; then
    echo 'Git directory is dirty. Please clean.'
    exit 1
fi
cd "$(git rev-parse --show-toplevel)"

NEW_VERSION=$1
git fetch
git checkout -B "$USER/rel/$NEW_VERSION" --track origin/main

find . -type f -name '*.toml' -exec sed -E -i '' 's/^version = "[[:digit:]]+\.[[:digit:]]+\.[[:digit:]]+"/version = "'"$NEW_VERSION"'"/g' {} +
cargo update
git add .
git diff --staged -- . ':(exclude)Cargo.lock'

read -p "Are you sure? " -n 1 -r
echo 
if [[ ! $REPLY =~ ^[Yy]$ ]]
then
    exit 1
fi

git commit -m "Updating version to $NEW_VERSION"
FORCE=""
if [[ $2 == "-f" ]]; then
    FORCE="-f"
fi

git tag $FORCE "v$NEW_VERSION"
git push $FORCE origin "v$NEW_VERSION"
git push $FORCE origin HEAD