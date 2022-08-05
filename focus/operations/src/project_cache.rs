// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use focus_internals::{model::repo::Repo, project_cache::ProjectCache};
use focus_util::app::{App, ExitCode};

pub fn push(
    app: Arc<App>,
    sparse_repo: impl AsRef<Path>,
    commit: String,
    shard_index: usize,
    shard_count: usize,
) -> Result<ExitCode> {
    if shard_index >= shard_count {
        anyhow::bail!("Shard index too high -- note that shard index is based at zero!")
    }
    if shard_count == 0 {
        anyhow::bail!("Shard count must be greater than zero!");
    }

    let repo = Repo::open(sparse_repo.as_ref(), app.clone())?;
    let object = repo
        .underlying()
        .revparse_single(&commit)
        .with_context(|| format!("Resolving commit {commit}"))?;
    let commit = object.as_commit().expect("Object was not a commit");
    let endpoint = repo
        .get_project_cache_remote_endpoint()?
        .ok_or_else(|| anyhow::anyhow!("Project cache remote endpoint not configured"))?;

    let cache = ProjectCache::new(&repo, endpoint, app)?;
    let keys = cache
        .generate_all(commit.id(), shard_index, shard_count)
        .context("Generating project cache data failed")?;
    let (_, build_graph_hash) = cache.get_build_graph_hash(commit.id(), true)?;

    cache
        .generate_and_push(&keys, &build_graph_hash, shard_index, shard_count)
        .context("Export failed")?;

    Ok(ExitCode(0))
}
