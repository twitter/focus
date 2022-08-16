#!/usr/bin/env bash
set -euxo pipefail

#Allow script to be run from anywhere
SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
CURRENT_DIR=$(realpath $(pwd))

# Setup clean fixtures
SCRIPT_NAME=$(basename "$0")
FIXTURE_DIR="focus-performance.fixture"
rm -rf "$FIXTURE_DIR"
mkdir -p "$FIXTURE_DIR"
cd "$FIXTURE_DIR"

git init source --bare
cd source
git remote add origin https://git.twitter.biz/ro/source --no-tags
# Only make the fixture as deep as the first branch. 0.0 branch is about 100 days older than 0.1
git ls-remote origin dbernadett/focus-performance-0.0 | awk '{print $1}' >> shallow
git config remote.origin.fetch '+refs/heads/dbernadett/focus-performance-0.*:refs/remotes/origin/dbernadett/focus-performance-0.*'
git fetch
git branch master origin/dbernadett/focus-performance-0.1 --no-track
cd ..
git clone file:///$CURRENT_DIR/$FIXTURE_DIR/source source-dense
