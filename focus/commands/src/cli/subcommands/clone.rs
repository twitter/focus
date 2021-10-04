use anyhow::Result;

use std::{path::PathBuf, sync::Arc};

use crate::{app::App, sparse_repos::Spec};

pub fn run(
    dense_repo: &PathBuf,
    sparse_repo: &PathBuf,
    branch: &String,
    spec: &Spec,
    filter_sparse: bool,
    all_branches: bool,
    generate_project_view: bool,
    app: Arc<App>,
) -> Result<()> {
    crate::sparse_repos::create_sparse_clone(
        dense_repo,
        sparse_repo,
        branch,
        spec,
        filter_sparse,
        all_branches,
        generate_project_view,
        app,
    )
}
