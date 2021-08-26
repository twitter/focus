use anyhow::Result;

use std::{path::PathBuf, sync::Arc};

use crate::{sandbox::Sandbox, sparse_repos::Spec};

pub fn run(
    name: &String,
    dense_repo: &PathBuf,
    sparse_repo: &PathBuf,
    branch: &String,
    spec: &Spec,
    filter_sparse: bool,
    sandbox: Arc<Sandbox>,
) -> Result<()> {
    crate::sparse_repos::create_sparse_clone(
        name,
        dense_repo,
        sparse_repo,
        branch,
        spec,
        filter_sparse,
        sandbox,
    )
}
