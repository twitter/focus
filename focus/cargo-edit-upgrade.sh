#!/bin/sh

set -euo pipefail errexit
set +x

for toml in `ls -1 */Cargo.toml`; do
        pushd $(dirname $toml)
        cargo upgrade
        popd
done
