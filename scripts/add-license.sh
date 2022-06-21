#!/bin/bash
set -euo pipefail

GOPATH=$(go env GOPATH || true)
if [[ "$GOPATH" != '' ]]; then
    export PATH="$PATH:$GOPATH/bin"
fi

if ! command -v addlicense &>/dev/null; then
    cat <<EOT
addlicense appears to not be installed.
Install with:

    go install github.com/google/addlicense@latest

EOT
    exit 1
fi

PROJECT_ROOT=$(realpath "$(dirname "$0")/..")
echo "Adding license to project at $PROJECT_ROOT"
addlicense -c "Twitter, Inc." -l "Apache-2.0" -s=only \
    -ignore '.git/**' \
    -ignore '.idea/**' \
    -ignore '.jj/**' \
    -ignore '.vscode/**' \
    -ignore 'target/**' \
    "$PROJECT_ROOT"
