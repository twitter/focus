use std::sync::Arc;
use std::{borrow::Borrow, fmt::Debug};

use crate::sandbox::Sandbox;
use anyhow::{Context, Result};
use std::time::SystemTime;
use tool_insights_client::Client;

#[must_use = "The exit code for the application should be returned and bubbled up to `main` so that it can be passed to `std::process::exit`."]
#[derive(Debug, PartialEq, Eq)]
pub struct ExitCode(pub i32);

#[derive(Clone)]
pub struct App {
    sandbox: Arc<Sandbox>,
    tool_insights_client: Client,
}

impl Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App").finish()
    }
}

impl App {
    pub fn new(preserve_sandbox_contents: bool, with_cmd_prefix: Option<&str>) -> Result<Self> {
        let invocation_description = std::env::args().collect::<Vec<String>>().join(" ");

        let sandbox = Arc::from(
            Sandbox::new(
                Some(&invocation_description),
                preserve_sandbox_contents,
                with_cmd_prefix,
            )
            .context("Failed to create sandbox")?,
        );
        let tool_insights_client = Client::new(
            env!("CARGO_PKG_NAME").to_owned(),
            env!("CARGO_PKG_VERSION").to_owned(),
            SystemTime::now(),
        );
        Ok(Self {
            sandbox,
            tool_insights_client,
        })
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
