# Focused Development

`focus` is a tool to manage [Git sparse checkouts](https://github.blog/2020-01-17-bring-your-monorepo-down-to-size-with-sparse-checkout/) derived from the [Bazel](https://bazel.build/) build graph.

# Installation

`focus` is written in [Rust](https://www.rust-lang.org/) and supports macOS and Linux. Git v2.35+ and Bazel need to be installed in the PATH env.

## MacOS Prerequisites
[Install Bazel](https://bazel.build/install/os-x)

[Install git > 2.35](https://formulae.brew.sh/formula/git)

WARN: If you run a `cargo test` you may run out of file descriptors. On MacOS you will need to use `ulimit -n X` to set a large file limit for the current shell.

TODO: Find instructions for increasing fd limit permanently.

## Linux Prerequisites
TODO: Install Prerequisites for Linux

## Common
[Install Rust](https://rustup.rs/), then install `focus` with

```
$ cargo install --locked --git https://github.com/twitter/focus
```

# Usage

If you are the repository administrator, first configure `focus` for your repo using the [Administration](focus/doc/administration.md) instructions.

For end-users, see [Usage](focus/doc/usage.md) for instructions on how to use `focus` to manage your sparse checkouts.

# Design

See various design documents at https://github.com/twitter/focus/tree/main/focus/doc. Focus will also be presented at [Git Merge 2022](https://git-merge.com/).
