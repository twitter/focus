#!/bin/bash

set -o errexit
fd -e cpp,h -e h -E third_party -x clang-format -i {}
