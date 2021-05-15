#!/bin/bash

set -euo pipefail
IFS=$'\n\t'

die() { echo "fatal: $*" >&2; exit 1; }

TEMP="$(mktemp -d 2>/dev/null || mktemp -d -t tempdir)" || die "failed to make tmpdir"
cleanup() { [[ -n "${TEMP:-}" ]] && rm -rf "${TEMP}"; }
trap cleanup EXIT

cd "$TEMP"

CHECKSUM="7154e88f5a8047aad4b80ebace58a059e36e7e2e4eb3b383127a28c711b4ff59"

wget -nv https://golang.org/dl/go1.16.4.linux-amd64.tar.gz
echo "$CHECKSUM go1.16.4.linux-amd64.tar.gz" |sha256sum --check

mkdir -p /go/roots
(
  cd /go/roots
  tar -zxf "$TEMP/go1.16.4.linux-amd64.tar.gz"
  mv go 1.16.4
)

mkdir -p /go/1.16.4
chown -R 1000:1000 /go
