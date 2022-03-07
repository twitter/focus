use crate::{app::App, sparse_repos};
use anyhow::Result;

use std::{path::PathBuf, sync::Arc};

pub fn run(
    dense_repo: PathBuf,
    sparse_repo: PathBuf,
    branch: String,
    coordinates: Vec<String>,
    layers: Vec<String>,
    copy_branches: bool,
    app: Arc<App>,
) -> Result<()> {
    sparse_repos::create_sparse_clone(
        dense_repo,
        sparse_repo,
        branch,
        coordinates,
        layers,
        copy_branches,
        app,
    )
}
