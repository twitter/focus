use anyhow::Result;

use std::{path::PathBuf, sync::Arc};

use crate::app::App;

pub fn run(
    dense_repo: PathBuf,
    sparse_repo: PathBuf,
    branch: String,
    coordinates: Vec<String>,
    layers: Vec<String>,
    copy_user_relevant_branches: bool,
    app: Arc<App>,
) -> Result<()> {
    crate::sparse_repos::create_sparse_clone(
        dense_repo,
        sparse_repo,
        branch,
        coordinates,
        layers,
        copy_user_relevant_branches,
        app,
    )
}
