#!/bin/bash
# Copyright 2022 Twitter, Inc.
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

# Run some basic checks to make sure that we are building off a clean state
if git status --porcelain | grep -q . ; then
    echo "Please clean your working tree  before building"
    exit 1
fi

set -x

exec env \
     TARGET=x86_64-apple-darwin \
     MACOSX_DEPLOYMENT_TARGET=10.15 \
     RUSTFLAGS="-Awarnings" \
     cargo build --features twttr --release
