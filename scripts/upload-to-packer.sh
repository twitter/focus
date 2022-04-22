#!/bin/bash

set -euo errexit
set -x

file="focus.$(uname -s).$(uname -m)"
test -d release && rm -r release
mkdir release
pushd release
mkdir bin
cp ../focus/target/release/focus bin/$file
tar jcf ../focus.tar.bz2 .
clusters=("smf1" "atla" "pdxa")
for cluster in ${clusters[@]}; do
    packer add_version --cluster=$cluster --use-tfe devprod $file ../focus.tar.bz2
    packer versions --cluster=$cluster --use-tfe devprod $file
done
popd
