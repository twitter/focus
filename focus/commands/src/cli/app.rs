use std::sync::Arc;

use crate::{ui::UserInterface, util::sandbox::Sandbox};
use anyhow::{Context, Result};

#[derive(Clone)]
pub struct App {
    ui: Arc<UserInterface>,
    sandbox: Arc<Sandbox>,
}

impl App {
    pub fn new(preserve_sandbox_contents: bool, interactive: bool) -> Result<Self> {
        let ui = Arc::from(UserInterface::new(interactive).context("Failed to start UI")?);
        let sandbox =
            Arc::from(Sandbox::new(preserve_sandbox_contents).context("Failed to create sandbox")?);

        Ok(Self { ui, sandbox })
    }

    /// Get a reference to the app's ui.
    pub fn ui(&self) -> Arc<UserInterface> {
        self.ui.clone()
    }

    /// Get a reference to the app's sandbox.
    pub fn sandbox(&self) -> Arc<Sandbox> {
        self.sandbox.clone()
    }
}
