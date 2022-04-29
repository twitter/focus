#!/bin/bash -e
# fetch and extract the rust toolchain

##
## Preconditions
##
which clang || echo "Please run this build with an LLVM toolchain present (e.g. under scl)" 1>&2

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
cp -rpv target/x86_64-unknown-linux-gnu/release/focus target/focus
popd

##
## Upload to Packer
##
file="focus.$(uname -s).$(uname -m)"
test -d release && rm -r release
mkdir release
pushd release
mkdir bin
cp ../focus/target/release/focus
tar jcf ../focus.tar.bz2 .
clusters=("smf1" "atla" "pdxa")
for cluster in ${clusters[@]}; do
    packer add_version --cluster=$cluster --use-tfe devprod $file ../focus.tar.bz2
    packer versions --cluster=$cluster --use-tfe devprod $file
done
popd

##
## Mark latest packer versions live
##
role=devprod
package="focus.Linux.x86_64"
clusters=("smf1" "atla" "pdxa")
for cluster in ${clusters[@]}; do
    version=$(packer versions --cluster=$cluster --use-tfe $role $package 2>&1 | grep 'Version' | awk '{print $2}' | tail -n1)
    echo "Latest version in $cluster is $version; marking it as LIVE"
    packer set_live --cluster=$cluster $role $package $version
done
