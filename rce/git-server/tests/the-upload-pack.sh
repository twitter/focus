#!/bin/bash
set -euo pipefail

export PAGER=cat

declare -a git_env
git_env=(
  'GIT_CONFIG_KEY_0=transfer.hideRefs'
  'GIT_CONFIG_VALUE_0=refs/tags'
  'GIT_CONFIG_KEY_1=transfer.hideRefs'
  'GIT_CONFIG_VALUE_1=refs/heads/bar'
  'GIT_CONFIG_KEY_2=transfer.hideRefs'
  'GIT_CONFIG_VALUE_2=refs/heads/bar/*'
  'GIT_CONFIG_COUNT=3'
)


exec /usr/bin/env "${git_env[@]}" git upload-pack "$@"
