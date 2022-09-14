# Using focus to build bazel
If you want to test-out focus, but don't have a monorepo to setup and test against, we recommend trying focus against the github.com/bazelbuild/bazel repo.

## Setup Steps
First you will need to follow the directions in the top-level README.md to install focus.

Then you will need to install `brew install openjdk@11`.

## Clone bazelbuild/bazel
If focus is not already in your path, you may want to temporarily add it with:

`export PATH="/Users/dbernadett/workspace/focus/target/debug:$PATH"`

You can then `focus new` a new sparse repo:

`focus new --dense-repo=https://github.com/bazelbuild/bazel --template bazel bazel-focus`
`cd bazel-focus`

## Checkout to build //src/main/cpp:blaze_util
Running `focus add bazel://src/main/cpp:blaze_util` will add the blaze_util and it's explicit dependencies.

However, immediately running `bazel build //src/main/cpp:blaze_util` will fail due to what appears to be missing implicit dependencies.

`focus add bazel://src/test/shell/bazel:list_source_repository.bzl` will fix the first error. Another `bazel build //src/main/cpp:blaze_util` will reveal a second missing implicit dependency which is `focus add bazel://src/main/res:winsdk_configure.bzl`.

Finally `bazel build //src/main/cpp:blaze_util` should succeed.

Congratulations on building a target without the whole bazelbuild/repo. We hope this exercise has sparked your curiosity.