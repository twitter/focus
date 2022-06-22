use anyhow::Context;
use anyhow::Result;
use focus_internals::model::repo::Repo;
use focus_util::app::{App, ExitCode};
use std::{path::Path, sync::Arc, time::Duration};

fn relative_time(current_commit_time: git2::Time, prospective_commit_time: git2::Time) -> String {
    let difference = prospective_commit_time.seconds() - current_commit_time.seconds();
    let difference_duration = Duration::from_secs(difference.abs() as u64);

    if difference > 0 {
        format!(
            "{} newer than",
            humantime::format_duration(difference_duration)
        )
    } else if difference < 0 {
        format!(
            "{} older than",
            humantime::format_duration(difference_duration)
        )
    } else {
        "the same as".to_string()
    }
}

pub fn run(sparse_repo: impl AsRef<Path>, app: Arc<App>) -> Result<ExitCode> {
    let repo = Repo::open(sparse_repo.as_ref(), app)?;
    let selections = repo.selection_manager()?;
    let selection = selections.selection()?;
    println!("{}", selection);

    if let Some(working_tree) = repo.working_tree() {
        if let Ok(head_commit) = working_tree.get_head_commit() {
            let primary_branch_name = repo.primary_branch_name()?;
            if let Ok(Some(prefetch_commit)) = repo
                .get_prefetch_head_commit("origin", &primary_branch_name.as_str())
                .context("Resolving prefetch head commit")
            {
                println!(
                    "The current commit is {} the upstream {} commit",
                    relative_time(head_commit.time(), prefetch_commit.time()),
                    &primary_branch_name
                )
            }
        }
    };

    Ok(ExitCode(0))
}
