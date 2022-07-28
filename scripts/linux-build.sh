#!/bin/bash -e
# Copyright 2022 Twitter, Inc.
# SPDX-License-Identifier: Apache-2.0

# fetch and extract the rust toolchain
set -eux

function die() {
    echo "$@" > /dev/stderr
    exit 1
}

role=${1-"the-focus-indexer"}
skip_mark_live=${2-"false"}

##
## Preconditions
##
which clang || die "Please run this build with an LLVM toolchain present (e.g. under scl)"

rm -rf toolchain
echo "Fetching toolchain..."
mkdir toolchain
pushd toolchain
packer fetch --use-tfe --cluster=smf1 io-perf rust-dev live
tar xzf rust-dev.tgz
popd

cargo_target=x86_64-unknown-linux-gnu
export PATH="${PATH}:${PWD}/toolchain/rust-dev/bin"
# get proxy credentials from TSS
echo "Getting proxy credentials..."
export HTTP_PROXY="http://rust-crates:$(cat /var/lib/tss/keys/io-perf/rust-crates/proxy-pass)@httpproxy.local.twitter.com:3128"
# configure cargo
echo "Configuring cargo..."
mkdir -p .cargo
echo "[http]" >> .cargo/config
echo "proxy = \"${HTTP_PROXY}\"" >> .cargo/config
# configure git
echo "Configuring git..."
git config http.proxy "${HTTP_PROXY}"
# toolchain info
echo "Toolchain info:"
rustc --version
cargo --version
# build
echo "Building..."
cargo build --release --target "$cargo_target"

##
## Upload to Packer
##
file="focus.$(uname -s).$(uname -m)"
test -d release && rm -r release
mkdir release
pushd release
mkdir bin
cp ../target/$cargo_target/release/focus bin/focus
tar jcf ../focus.tar.bz2 .
clusters=("smf1" "atla" "pdxa")
for cluster in ${clusters[@]}; do
    packer add_version "--cluster=$cluster" --use-tfe "$role" "$file" ../focus.tar.bz2
    packer versions "--cluster=$cluster" --use-tfe "$role" "$file"
done
popd

##
## Mark latest packer versions live
##
if [[ "$skip_mark_live" == "false" ]]; then
    package="focus.Linux.x86_64"
    clusters=("smf1" "atla" "pdxa")
    for cluster in ${clusters[@]}; do
        version=$(packer versions "--cluster=$cluster" --use-tfe "$role" "$package" 2>&1 | grep 'Version' | awk '{print $2}' | tail -n1)
        echo "Latest version in $cluster is $version; marking it as LIVE"
        packer set_live "--cluster=$cluster" "$role" "$package" "$version"
    done
fi