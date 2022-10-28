# Focused Development

`focus` is a tool to manage [Git sparse checkouts](https://github.blog/2020-01-17-bring-your-monorepo-down-to-size-with-sparse-checkout/) derived from the [Bazel](https://bazel.build/) build graph.

# Installation

`focus` is written in [Rust](https://www.rust-lang.org/) and supports macOS and Linux. Git v2.35+ and Bazel need to be installed in the PATH env. General 

## MacOS Prerequisites
[Install Bazel](https://bazel.build/install/os-x)

[Install git > 2.35](https://formulae.brew.sh/formula/git)

WARN: If you run a `cargo test` you may run out of file descriptors. On MacOS you will need to use `ulimit -n X` to set a large file limit for the current shell. On macOS Big Sur, you can write a plist to do this permanently:
```
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple/DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
  <plist version="1.0">
    <dict>
      <key>Label</key>
        <string>limit.maxfiles</string>
      <key>ProgramArguments</key>
        <array>
	  <string>sudo</string>
          <string>launchctl</string>
          <string>limit</string>
          <string>maxfiles</string>
          <string>655360</string>
          <string>1048576</string>
        </array>
      <key>RunAtLoad</key>
        <true />
    </dict>
  </plist>
``` 
then load it with `sudo launchctl load -w /Library/LaunchDaemons/limit.maxfiles.plist`.

Note: these instructions are from a GitHub issue https://github.com/gradle/gradle/issues/17274. Thanks to those folks.


## Linux Prerequisites

Git 2.35+: get this through your distro's package manager or [download pre-built binaries or sources here](https://git-scm.com/downloads).
You'll need Bazel installed. See [these instructions](https://bazel.build/install/) for details on how to install on mainstream Linux distros. Alternative distros probably have a reasonable Bazel package available by now.


## Common
[Install Rust](https://rustup.rs/), then install `focus` with

```
$ cargo install --locked --git https://github.com/twitter/focus
```

# Usage

If you are the repository administrator, first configure `focus` for your repo using the [Administration](focus/doc/administration.md) instructions.

For end-users, see [Usage](focus/doc/usage.md) for instructions on how to use `focus` to manage your sparse checkouts.

For new or curious users, see [Bazel Tutorial](focus/doc/bazel_tutorial.md) for instructions on how to try `focus` on the [bazel repository](https://github.com/bazelbuild/bazel) itself.

# Design

See various design documents at https://github.com/twitter/focus/tree/main/focus/doc. Focus was presented at [Git Merge 2022](https://git-merge.com/); [see the slides here](https://docs.google.com/presentation/d/12RVWPIms-rFKfteqYa5bpSWElIiJ1oCAYobLb5DihQo/edit?usp=sharing).
