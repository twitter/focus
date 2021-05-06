#!/bin/bash
set -euo pipefail

die() { printf 'fatal: %s\n' "$*"; exit 1; }

if [[ -z "$TEST_TEMPDIR" ]]; then
  die "you must set TEST_TEMPDIR in the environment"
fi

if [[ -z "$CONFIG_FILE" ]]; then
  die "you must set CONFIG_FILE in the environment"
fi

cd "$TEST_TEMPDIR"

ORIGIN_URL="file://${TEST_ORIGIN}"

git init "${TEST_REPO}"
git init admin
git init --bare "${TEST_ORIGIN}"

(
  cd "${TEST_REPO}"
  echo 'anfile' > afile
  git add afile
  git commit -am 'initial commit'
  git remote add origin "${ORIGIN_URL}"
  git push -u origin master
  git config --local "twgit.admin.localref" "refs/admin/twgit"
  git config --local "twgit.admin.remoteref" "refs/admin/twgit"
  git config --local "twgit.admin.remotename" "origin"
  git config --local "twgit.admin.updateinterval" "15m0s"
  git config --local "twgit.admin.blobpath" "$(basename "$CONFIG_FILE")"
)

(
  cd admin
  cp "${CONFIG_FILE}" .
  git add .
  git commit -am 'added config file'
  git push "${ORIGIN_URL}" HEAD:refs/admin/twgit
)
