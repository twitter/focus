// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};

use focus_internals::model::repo::Repo;
use focus_util::{app::App, git_helper::ConfigExt};
use tracing::error;

pub fn list(app: Arc<App>, sparse_repo_path: PathBuf, remote_name: &str) -> Result<()> {
    let repo = Repo::open(&sparse_repo_path, app).context("Failed to open repo")?;
    let underlying_repo = repo.underlying();
    let config = underlying_repo.config().with_context(|| {
        format!(
            "Could not get config for sparse repo {}",
            sparse_repo_path.to_string_lossy()
        )
    })?;
    let fetchspecs = config
        .multivar_values(format!("remote.{}.fetch", remote_name), None)
        .context("Could not get refspecs from git config")?;

    let branch_names =
        get_ref_names_from_refspecs(fetchspecs.iter().map(|spec| spec.as_str()).collect())
            .context("Could not determine ref names from refspecs in config")?;
    for branch in branch_names {
        println!("{}", branch);
    }

    Ok(())
}

pub fn search(
    app: Arc<App>,
    sparse_repo_path: PathBuf,
    remote_name: &str,
    search_term: &str,
) -> Result<()> {
    let repo = Repo::open(&sparse_repo_path, app).context("Failed to open repo")?;
    let underlying_repo = repo.underlying();
    let mut remote = underlying_repo
        .find_remote(remote_name)
        .with_context(|| format!("Could not find remote named {}", remote_name))?;
    remote
        .connect(git2::Direction::Fetch)
        .with_context(|| format!("Could not connect to remote {}", remote_name))?;

    let ref_names = filter_ref_names_from_remote(&remote, search_term)
        .context("Could not get ref names from remote")?;

    for ref_name in ref_names {
        println!("{}", ref_name);
    }

    Ok(())
}

fn filter_ref_names_from_remote<'a>(
    remote: &'a git2::Remote,
    search_term: &str,
) -> Result<Vec<&'a str>> {
    let refs = remote
        .list()
        .with_context(|| format!("Could not list remote refs from {}", remote.name().unwrap()))?;
    let filtered_refs = refs
        .iter()
        .map(|ref_locations| ref_locations.name())
        .filter(|name| name.contains(search_term));
    let ref_names = filtered_refs
        .filter_map(get_ref_names_from_ref_location)
        .collect();

    Ok(ref_names)
}

fn get_ref_names_from_refspecs(refspecs: Vec<&str>) -> Result<Vec<&str>> {
    Ok(refspecs
        .iter()
        .map(|refspec| {
            let split: Vec<&str> = refspec.split(':').collect();
            let ref_location = match split.as_slice() {
                [local, _] => local,
                _ => {
                    let a = &"Malformed branch configuration";
                    error!("{} {}", &a, refspec);
                    a
                }
            };
            ref_location.to_owned()
        })
        .filter_map(get_ref_names_from_ref_location)
        .collect())
}

/// Gets ref name or ref prefix from names like 'refs/heads/master' or '+refs/heads/user/*'.
fn get_ref_names_from_ref_location(ref_name: &str) -> Option<&str> {
    let ref_name_no_ff_token = ref_name.strip_prefix('+').unwrap_or(ref_name);
    ref_name_no_ff_token.strip_prefix("refs/heads/")
}

#[cfg(test)]
mod testing {
    use focus_testing::ScratchGitRepo;
    use git2::Repository;

    use super::*;

    #[test]
    fn test_get_ref_names_from_ref_locations() -> anyhow::Result<()> {
        let ref_names: Vec<&str> = vec!["refs/heads/master", "refs/heads/test/*"]
            .iter()
            .filter_map(|&name| get_ref_names_from_ref_location(name))
            .collect();

        assert_eq!(vec!["master", "test/*"], ref_names);

        Ok(())
    }

    #[test]
    fn test_get_ref_names_and_prefixes_from_refspecs() -> anyhow::Result<()> {
        let refspecs: Vec<&str> = vec![
            "refs/heads/master:refs/remotes/origin/master",
            "refs/heads/test/*:refs/remotes/origin/test/*",
        ];
        let ref_names = get_ref_names_from_refspecs(refspecs)?;

        assert_eq!(vec!["master", "test/*"], ref_names);

        Ok(())
    }

    #[test]
    fn test_filter_ref_names_from_remote() -> anyhow::Result<()> {
        let temp_remote_dir = tempfile::tempdir()?;
        let scratch_remote = ScratchGitRepo::new_static_fixture(temp_remote_dir.path())?;
        scratch_remote.create_and_switch_to_branch("test/one")?;
        scratch_remote.create_and_switch_to_branch("team/test")?;
        scratch_remote.create_and_switch_to_branch("other")?;

        let temp_sparse_dir = tempfile::tempdir()?;
        let scratch_sparse = ScratchGitRepo::new_static_fixture(temp_sparse_dir.path())?;
        let sparse_repo = Repository::open(scratch_sparse.path())?;
        sparse_repo.remote(
            "origin",
            &format!("file://{}", scratch_remote.path().to_str().unwrap()),
        )?;
        let mut remote = sparse_repo.find_remote("origin")?;
        remote.connect(git2::Direction::Fetch)?;

        assert_eq!(
            filter_ref_names_from_remote(&remote, "test")?,
            vec!["team/test", "test/one"]
        );
        Ok(())
    }
}
