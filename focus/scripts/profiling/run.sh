#!/bin/bash

set -euo pipefail

if [ "$EUID" -ne 0 ]; then
    echo "Please run this script as superuser (e.g. via sudo)" 1>&2
    exit 1
fi

files_to_copy="Dockerfile vbatts-bazel-epel-7.repo"

name="twitter_ee_focus_profiling"
dir=$(mktemp -d -t "$name.XXXXXX")

cleanup() {
    test -d $dir && rm -rf $dir
}
trap cleanup EXIT

sudo -u $USER cp -rv $files_to_copy $dir


set -x

docker build \
     --tag $name \
     $dir

exec docker run \
     --rm \
     -it \
     --cap-drop=all \
     --security-opt no-new-privileges \
     --cpus 4 \
     --memory=8192m \
     --memory-swap=0m \
     --memory-swappiness=0 \
     --tmpfs /home/dev/rust:exec,size=256m \
     --tmpfs /home/dev/.cargo:size=256m \
     --tmpfs /tmp:size=64m \
     --security-opt seccomp=seccomp-perf.json \
     -v $(pwd):/home/build/src \
     -v $HOME/workspace:/home/build/workspace \
     $name \
     $@
#     -u profiler_user \
     # --read-only \
