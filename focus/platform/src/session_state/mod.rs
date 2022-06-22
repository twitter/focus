// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

/// An enumeration representing whether the computer is being actively used.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    /// The computer is being actively used.
    Active,

    /// The computer has not been interacted with for the given duration.
    Idle,

    /// It is not possible to determine whether the computer is being interacted with.
    Unknown,
}

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "macos")]
pub use macos::has_session_been_idle_for;

/// Default implementation
#[cfg(not(target_os = "macos"))]
fn has_session_been_idle_for(_at_least: Duration) -> SessionStatus {
    SessionStatus::Unknown
}
