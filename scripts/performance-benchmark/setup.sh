#!/usr/bin/env bash
set -euxo pipefail

# Allow script to be run from anywhere
SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
CURRENT_DIR=$(realpath $(pwd))

# Get Arguments
if [ -z "${FOCUS_PERF_REMOTE}" ]; then
    echo "FOCUS_PERF_REMOTE must be set"
    exit
fi
if [[ -z "${FOCUS_PERF_REF_PREFIX}" ]]; then
    echo "FOCUS_PERF_REF_PREFIX must be set"
    exit
fi

# Setup clean fixtures
SCRIPT_NAME=$(basename "$0")
FIXTURE_DIR="focus-performance.fixture"
rm -rf "$FIXTURE_DIR"
mkdir -p "$FIXTURE_DIR"
cd "$FIXTURE_DIR"

git init source --bare
cd source
git remote add origin "$FOCUS_PERF_REMOTE" --no-tags
# Only make the fixture as deep as the first branch. 0.0 branch is about 100 days older than 0.1
git ls-remote origin "${FOCUS_PERF_REF_PREFIX}0" | awk '{print $1}' >> shallow
git config remote.origin.fetch "+refs/heads/${FOCUS_PERF_REF_PREFIX}*:refs/remotes/origin/${FOCUS_PERF_REF_PREFIX}*"
git fetch
git branch master "origin/${FOCUS_PERF_REF_PREFIX}1" --no-track
cd ..
git clone file:///$CURRENT_DIR/$FIXTURE_DIR/source source-dense
