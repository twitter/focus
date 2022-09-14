# Focused Development

`focus` is a tool to manage [Git sparse checkouts](https://github.blog/2020-01-17-bring-your-monorepo-down-to-size-with-sparse-checkout/) derived from the [Bazel](https://bazel.build/) build graph.

# Installation

`focus` is written in [Rust](https://www.rust-lang.org/) and supports macOS and Linux. [Install Rust](https://rustup.rs/), then install `focus` with

```
$ cargo install --locked --git https://github.com/twitter/focus
```

# Usage

If you are the repository administrator, first configure `focus` for your repo using the [Administration](focus/doc/administration.md) instructions.

For end-users, see [Usage](focus/doc/usage.md) for instructions on how to use `focus` to manage your sparse checkouts.

For new or curious users, see [Bazel Tutorial](focus/doc/bazel_tutorial.md) for instructions on how to try `focus` on the [bazel repository](https://github.com/bazelbuild/bazel) itself.

# Design

See various design documents at https://github.com/twitter/focus/tree/main/focus/doc. Focus will also be presented at [Git Merge 2022](https://git-merge.com/).
