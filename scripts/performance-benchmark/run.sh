#!/usr/bin/env bash
# Copyright 2022 Twitter, Inc.
# SPDX-License-Identifier: Apache-2.0

set -euxo pipefail

#Allow script to be run from anywhere
SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

# Setup a clean sandbox
SCRIPT_NAME=$(basename "$0")
SANDBOX_DIR="$SCRIPT_NAME.sandbox"

# Get Arguments
if [[ -z "${FOCUS_PERF_REF_PREFIX}" ]]; then
    echo "FOCUS_PERF_REF_PREFIX must be set"
    exit
fi
if [[ -z "${FOCUS_REPO_DIR}" ]]; then
    echo "FOCUS_REPO_DIR must be set"
    exit
fi

mkdir -p trash
mkdir -p "$SANDBOX_DIR"

# remove sandbox in the background
RANDOM_NAME="$(dd if=/dev/urandom count=8 bs=1 | xxd -p)"
mv "$SANDBOX_DIR" "trash/$RANDOM_NAME"
nohup rm -rf trash/"$RANDOM_NAME" &>/dev/null &

mkdir -p "$SANDBOX_DIR"

FIXTURE_DIR="focus-performance.fixture"
# Reset the remote
git -C "$FIXTURE_DIR/source" branch -f master "origin/${FOCUS_PERF_REF_PREFIX}1"

cd "$SANDBOX_DIR"

# Bash function defined:
focus.devb() {
  "${FOCUS_REPO_DIR}/target/debug/focus" $@
}

# May need to specify some projects after focus new
rm -rf source-focused
echo "focus new:" > timings.txt
date >> timings.txt
focus.devb new --dense-repo="../$FIXTURE_DIR/source-dense" source-focused 2> >(tee focus.new.1)
date >> timings.txt

cd source-focused

# Test sync after pull
git -C "../../$FIXTURE_DIR/source" branch -f master "origin/${FOCUS_PERF_REF_PREFIX}2"
git pull
echo "focus sync to ${FOCUS_PERF_REF_PREFIX}2:" >> ../timings.txt
# We may not need to call sync explicitly, if the git pull is working
date >> ../timings.txt
focus.devb sync 2> >(tee ../focus.sync.1)
date >> ../timings.txt

# Test sync after pull
git -C "../../$FIXTURE_DIR/source" branch -f master "origin/${FOCUS_PERF_REF_PREFIX}3"
git pull
echo "focus sync to ${FOCUS_PERF_REF_PREFIX}3:" >> ../timings.txt
# We may not need to call sync explicitly, if the git pull is working
date >> ../timings.txt
focus.devb sync 2> >(tee ../focus.sync.2)
date >> ../timings.txt


