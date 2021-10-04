use anyhow::Result;

use std::{path::PathBuf, sync::Arc};

use crate::app::App;

pub fn run(
    dense_repo: PathBuf,
    sparse_repo: PathBuf,
    branch: String,
    coordinates: Vec<String>,
    layers: Vec<String>,
    app: Arc<App>,
) -> Result<()> {
    crate::sparse_repos::create_sparse_clone(
        dense_repo,
        sparse_repo,
        branch,
        coordinates,
        layers,
        app,
    )
}
