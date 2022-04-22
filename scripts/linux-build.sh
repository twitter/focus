#!/bin/bash -e
# fetch and extract the rust toolchain
rm -rf toolchain
echo "Fetching toolchain..."
mkdir toolchain
pushd toolchain
packer fetch --use-tfe --cluster=smf1 io-perf rust-dev live
tar xzf rust-dev.tgz
popd

export PATH=${PATH}:${PWD}/toolchain/rust-dev/bin
rm -rf focus
git clone --single-branch -b main --depth 1 http://git.twitter.biz/ro/focus
pushd focus
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
git config http.proxy ${HTTP_PROXY}
# toolchain info
echo "Toolchain info:"
rustc --version
cargo --version
# build
echo "Building..."
cargo build --release --target x86_64-unknown-linux-gnu
# copy build artifacts to common location
cp -rpv target/x86_64-unknown-linux-gnu/release target/
popd
