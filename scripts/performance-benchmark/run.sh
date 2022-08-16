#!/usr/bin/env bash
set -euxo pipefail

#Allow script to be run from anywhere
SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )

# Setup a clean sandbox
SCRIPT_NAME=$(basename "$0")
SANDBOX_DIR="$SCRIPT_NAME.sandbox"

# rm -rf "$SANDBOX_DIR"
mkdir -p trash
mkdir -p "$SANDBOX_DIR"

# remove sandbox in the background
mv "$SANDBOX_DIR" trash/.
nohup rm -rf trash/"$SANDBOX_DIR" &>/dev/null &

mkdir -p "$SANDBOX_DIR"

FIXTURE_DIR="focus-performance.fixture"
# Reset the remote
git -C "$FIXTURE_DIR/source" branch -f master origin/dbernadett/focus-performance-0.1

# Get a clean dense repo
#cp -r -P "$FIXTURE_DIR/source-dense" "$SANDBOX_DIR/."

cd "$SANDBOX_DIR"

# Bash function defined:
focus.devb() {
  ~/workspace/focus/target/debug/focus $@
}

# May need to specify some projects after focus new
rm -rf source-focused
echo "focus new:" > timings.txt
date >> timings.txt
time focus.devb new --dense-repo="../$FIXTURE_DIR/source-dense" source-focused
date >> timings.txt

cd source-focused

# Test sync after pull
git -C "../../$FIXTURE_DIR/source" branch -f master origin/dbernadett/focus-performance-0.2 
git pull
echo "focus sync to focus-performance-0.2:" >> ../timings.txt
# We may not need to call sync explicitly, if the git pull is working
date >> ../timings.txt
focus.devb sync
date >> ../timings.txt

# Test sync after pull
git -C "../../$FIXTURE_DIR/source" branch -f master origin/dbernadett/focus-performance-0.3
git pull
echo "focus sync to focus-performance-0.3:" >> ../timings.txt
# We may not need to call sync explicitly, if the git pull is working
date >> ../timings.txt
focus.devb sync
date >> ../timings.txt


