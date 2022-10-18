// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use focus_internals::{model::repo::Repo, target::TargetTypes};
use focus_util::app::{App, ExitCode};
use std::{collections::HashSet, path::Path, sync::Arc};

pub fn run(
    sparse_repo: impl AsRef<Path>,
    app: Arc<App>,
    targets_flag: bool,
    target_types: Vec<TargetTypes>,
) -> Result<ExitCode> {
    let target_types = HashSet::<TargetTypes>::from_iter(target_types.iter().cloned());
    let repo = Repo::open(sparse_repo.as_ref(), app)?;
    let selections = repo.selection_manager()?;
    let selection = selections.selection()?;
    if target_types.is_empty() && !targets_flag {
        println!("{}", selection);
    } else {
        let mut targets = selection.targets;
        if !targets_flag {
            for project in selection.projects {
                println!("{}", project.name)
            }
        } else {
            targets = selections.compute_complete_target_set()?;
        }

        for target in targets {
            match target {
                focus_internals::target::Target::Bazel(_) => {
                    if target_types.contains(&TargetTypes::Bazel) {
                        println!("{}", target);
                    }
                }
                focus_internals::target::Target::Directory(_) => {
                    if target_types.contains(&TargetTypes::Directory) {
                        println!("{}", target);
                    }
                }
            }
        }
    }

    Ok(ExitCode(0))
}
