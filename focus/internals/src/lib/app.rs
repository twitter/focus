use std::borrow::Borrow;
use std::sync::Arc;

use crate::{ui::UserInterface, util::sandbox::Sandbox};
use anyhow::{Context, Result};
use std::time::SystemTime;
use ti_library::tool_insights_client::ToolInsightsClient;

#[derive(Clone)]
pub struct App {
    ui: Arc<UserInterface>,
    sandbox: Arc<Sandbox>,
    tool_insights_client: ToolInsightsClient,
}

impl App {
    pub fn new(preserve_sandbox_contents: bool, interactive: bool) -> Result<Self> {
        let ui = Arc::from(UserInterface::new(interactive).context("Failed to start UI")?);
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
            ui,
            sandbox,
            tool_insights_client,
        })
    }

    /// Get a reference to the app's ui.
    pub fn ui(&self) -> Arc<UserInterface> {
        self.ui.clone()
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
