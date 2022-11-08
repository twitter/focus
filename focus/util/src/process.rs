// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{ffi::OsStr, path::PathBuf, process::Command};

/// Returns a textual description of the current process
pub fn get_process_description() -> String {
    let mut cmdline = String::new();
    for arg in std::env::args() {
        cmdline.push_str(arg.as_str());
        cmdline.push(' ');
    }
    if cmdline.ends_with(' ') {
        cmdline.pop();
    }
    let current_dir =
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("unknown-directory"));
    format!(
        "'{}' running in directory {:?} with PID {} started by {} on host {}",
        cmdline,
        current_dir,
        std::process::id(),
        whoami::username(),
        whoami::hostname()
    )
}

pub fn pretty_print_command<'cmd>(command: &'cmd mut Command) -> String {
    let convert_os_str =
        |s: &'cmd OsStr| -> &'cmd str { s.to_str().unwrap_or("<???>").trim_matches('"') };

    let mut buf = convert_os_str(command.get_program()).to_owned();
    for arg in command.get_args() {
        buf.push(' ');
        buf.push_str(convert_os_str(arg));
    }
    buf
}
