#!/bin/bash
# Copyright 2022 Twitter, Inc.
# SPDX-License-Identifier: Apache-2.0


set -eu
set -o pipefail

set -x

exec env \
     TARGET=x86_64-apple-darwin \
     MACOSX_DEPLOYMENT_TARGET=10.15 \
     RUSTFLAGS="-Awarnings" \
     cargo build --features twttr --release
