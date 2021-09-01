use anyhow::Result;

use std::{path::PathBuf, sync::Arc};

use crate::{sandbox::Sandbox, sparse_repos::Spec};

pub fn run(
    dense_repo: &PathBuf,
    sparse_repo: &PathBuf,
    branch: &String,
    spec: &Spec,
    filter_sparse: bool,
    generate_project_view: bool,
    sandbox: Arc<Sandbox>,
) -> Result<()> {
    crate::sparse_repos::create_sparse_clone(
        dense_repo,
        sparse_repo,
        branch,
        spec,
        filter_sparse,
        generate_project_view,
        sandbox,
    )
}
