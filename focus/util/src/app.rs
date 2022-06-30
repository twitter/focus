// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use std::{borrow::Borrow, fmt::Debug};

use crate::sandbox::Sandbox;
use anyhow::{Context, Result};
use focus_testing::GitBinary;
use std::time::SystemTime;
use tool_insights_client::Client;

#[must_use = "The exit code for the application should be returned and bubbled up to `main` so that it can be passed to `std::process::exit`."]
#[derive(Debug, PartialEq, Eq)]
pub struct ExitCode(pub i32);

#[derive(Clone)]
pub struct App {
    git_binary: GitBinary,
    sandbox: Arc<Sandbox>,
    tool_insights_client: Client,
}

impl Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App").finish()
    }
}

impl App {
    pub fn new_for_testing() -> Result<Self> {
        Self::new(false, None, None, None)
    }
    pub fn new(
        preserve_sandbox_contents: bool,
        with_cmd_prefix: Option<&str>,
        app_name: Option<String>,
        app_version: Option<String>,
    ) -> Result<Self> {
        let git_binary = GitBinary::from_env()?;
        let sandbox = Arc::from(
            Sandbox::new(preserve_sandbox_contents, with_cmd_prefix)
                .context("Failed to create sandbox")?,
        );
        let tool_insights_client = Client::new(
            app_name.unwrap_or_else(|| env!("CARGO_PKG_NAME").to_owned()),
            app_version.unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned()),
            SystemTime::now(),
        );
        Ok(Self {
            git_binary,
            sandbox,
            tool_insights_client,
        })
    }

    /// Get a reference to the Git binary that this app is using.
    pub fn git_binary(&self) -> &GitBinary {
        &self.git_binary
    }

    /// Get a reference to the app's sandbox.
    pub fn sandbox(&self) -> Arc<Sandbox> {
        self.sandbox.clone()
    }

    /// Get a reference to the app's tool-insights client.
    pub fn tool_insights_client(&self) -> &Client {
        self.tool_insights_client.borrow()
    }
}
