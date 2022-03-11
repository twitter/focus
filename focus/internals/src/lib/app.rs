use std::sync::Arc;
use std::{borrow::Borrow, fmt::Debug};

use crate::util::sandbox::Sandbox;
use anyhow::{Context, Result};
use std::time::SystemTime;
use ti_library::tool_insights_client::ToolInsightsClient;

#[must_use = "The exit code for the application should be returned and bubbled up to `main` so that it can be passed to `std::process::exit`."]
#[derive(Debug, PartialEq, Eq)]
pub struct ExitCode(pub i32);

#[derive(Clone)]
pub struct App {
    sandbox: Arc<Sandbox>,
    tool_insights_client: ToolInsightsClient,
}

impl Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App").finish()
    }
}

impl App {
    pub fn new(preserve_sandbox_contents: bool) -> Result<Self> {
        let sandbox =
            Arc::from(Sandbox::new(preserve_sandbox_contents).context("Failed to create sandbox")?);
        let tool_insights_client = ToolInsightsClient::new(
            // TODO: get this from toml file
            "focus".to_string(),
            // TODO: get this from toml file
            "0.3".to_string(),
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
    pub fn tool_insights_client(&self) -> &ToolInsightsClient {
        self.tool_insights_client.borrow()
    }
}
