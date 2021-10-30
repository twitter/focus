use std::{path::Path, sync::Arc};

use anyhow::{bail, Context, Result};

use focus_internals::app::App;

pub fn run(listen_address: String, repos: &Path, _app: Arc<App>) -> Result<()> {
    if let Err(e) = focus_internals::server::run(listen_address.as_str()) {
        bail!("Running server failed: {}", e);
    }

    Ok(())
}
