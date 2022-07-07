//! Run with
//!
//! ```bash
//! cargo run --example calc_invalidation_rate -- ~/workspace/path/to/repo 10000
//! ```

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use clap::Parser;
use focus_internals::{
    index::{content_hash_dependency_key, ContentHash, DependencyKey, HashContext},
    model::repo::Repo,
    target::{Target, TargetSet},
};
use focus_util::{app::App, git_helper};

#[derive(Parser, Debug)]
struct Opts {
    sparse_repo_path: PathBuf,

    /// The number of commits backward from the `HEAD` commit to sample.
    num_commits: usize,
}

fn average(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut sum: f64 = 0.0;
    let mut len: f64 = 0.0;
    for value in values {
        sum += value as f64;
        len += 1.0;
    }
    sum / len
}

fn main() -> anyhow::Result<()> {
    let Opts {
        sparse_repo_path,
        num_commits,
    } = Opts::parse();

    let app = Arc::new(App::new(false, None, None, None)?);
    let repo = Repo::open(&sparse_repo_path, app.clone())?;
    let selections = repo.selection_manager()?;
    let all_targets = {
        let mut targets = TargetSet::try_from(&selections.project_catalog().mandatory_projects)?;
        targets.extend(TargetSet::try_from(
            &selections.project_catalog().optional_projects,
        )?);
        targets
    };

    #[derive(Clone, Debug)]
    struct HashChangeInfo {
        current_hash: ContentHash,
        commits_since_last_hash_change: usize,
    }
    let mut target_stats: HashMap<Target, Vec<HashChangeInfo>> = all_targets
        .into_iter()
        .map(|target| (target, Vec::new()))
        .collect();

    let repo = git2::Repository::open(sparse_repo_path)?;
    let mut commit = git_helper::get_head_commit(&repo)?;
    for i in 0..num_commits {
        eprintln!(
            "Hashing {i}/{num_commits} {:?}",
            commit.summary().unwrap_or_default()
        );
        let tree = commit.tree()?;
        let hash_context = HashContext {
            repo: &repo,
            head_tree: &tree,
            caches: Default::default(),
        };

        for (target, hash_change_infos) in target_stats.iter_mut() {
            let dep_key = DependencyKey::from(target.clone());
            let hash = content_hash_dependency_key(&hash_context, &dep_key, &mut HashSet::new())?;
            match hash_change_infos.last_mut() {
                Some(hash_change_info) if hash_change_info.current_hash == hash => {
                    hash_change_info.commits_since_last_hash_change += 1;
                }
                _ => {
                    hash_change_infos.push(HashChangeInfo {
                        current_hash: hash,
                        commits_since_last_hash_change: 0,
                    });
                }
            }
        }
        commit = commit.parent(0)?;
    }

    let mut averages: Vec<_> = target_stats
        .iter()
        .map(|(target, hash_change_infos)| {
            let changes: Vec<_> = hash_change_infos
                .iter()
                .map(|hash_change_info| hash_change_info.commits_since_last_hash_change as f64)
                .collect();
            (target, average(changes))
        })
        .collect();
    averages.sort_by(|(_, lhs), (_, rhs)| lhs.partial_cmp(rhs).unwrap());
    println!("Most churning targets:");
    for (target, value) in &averages[..10] {
        println!("{target} {value:.2}");
    }

    let average_churn = average(averages.iter().map(|(_, value)| *value));
    println!("Average churn rate: {average_churn:.2}");

    Ok(())
}
