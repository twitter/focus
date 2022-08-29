#!/bin/bash

set -eu
set -o pipefail

set -x

exec env \
     TARGET=x86_64-apple-darwin \
     MACOSX_DEPLOYMENT_TARGET=10.15 \
     RUSTFLAGS="-Awarnings" \
     cargo build --release
