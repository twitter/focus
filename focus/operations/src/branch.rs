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
    let config = underlying_repo.config()?;
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
            "+refs/heads/allow-ff/*:refs/remotes/origin/allow-ff/*",
        ];
        let ref_names = get_ref_names_from_refspecs(refspecs)?;

        assert_eq!(vec!["master", "test/*", "allow-ff/*"], ref_names);

        Ok(())
    }
}
