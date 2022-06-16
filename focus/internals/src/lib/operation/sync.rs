use crate::model::repo::Repo;
use crate::{index::RocksDBMemoizationCacheExt, locking};

use crate::operation::util::perform;
use crate::target::TargetSet;
use content_addressed_cache::RocksDBCache;
use focus_util::app::App;
use focus_util::backed_up_file::BackedUpFile;
use tracing::warn;

use std::convert::TryFrom;

use std::path::Path;

use std::sync::Arc;

use anyhow::{bail, Context, Result};

pub struct SyncResult {
    pub checked_out: bool,
    pub commit_id: Option<git2::Oid>,
    pub skipped: bool,
}

/// Synchronize the sparse repo's contents with the build graph. Returns whether a checkout actually occured.
pub fn run(sparse_repo: &Path, preemptive: bool, app: Arc<App>) -> Result<SyncResult> {
    let repo = Repo::open(sparse_repo, app.clone()).context("Failed to open the repo")?;

    let _lock = locking::hold_lock(sparse_repo, Path::new("sync.lock"))
        .context("Failed to obtain synchronization lock")?;

    let sparse_profile_path = repo.git_dir().join("info").join("sparse-checkout");
    if !sparse_profile_path.is_file() {
        bail!("This does not appear to be a focused repo -- it is missing a sparse checkout file");
    }

    if preemptive && !repo.get_preemptive_sync_enabled()? {
        return Ok(SyncResult {
            checked_out: false,
            commit_id: None,
            skipped: true,
        });
    }

    let selections = repo.selection_manager()?;
    let selection = selections.computed_selection()?;
    let targets = TargetSet::try_from(&selection).context("constructing target set")?;

    // Add target/project to TI data.
    let app_for_ti_client = app.clone();
    let ti_client = app_for_ti_client.tool_insights_client();
    ti_client.get_context().add_to_custom_map(
        "sync_kind",
        if preemptive {
            "preemptive"
        } else {
            "immediate"
        },
    );

    let backed_up_sparse_profile: Option<BackedUpFile> = if preemptive {
        None
    } else {
        super::ensure_clean::run(sparse_repo, app.clone())
            .context("Failed trying to determine whether working trees were clean")?;

        ti_client
            .get_context()
            .add_to_custom_map("total_target_count", targets.len().to_string());
        ti_client.get_context().add_to_custom_map(
            "user_selected_project_count",
            selection.projects.len().to_string(),
        );
        ti_client.get_context().add_to_custom_map(
            "user_selected_target_count",
            selection.targets.len().to_string(),
        );

        Some(BackedUpFile::new(&sparse_profile_path)?)
    };

    let head_commit = repo.get_head_commit().context("Resolving head commit")?;

    // Figure out if this repo has a "master" branch or "main" branch.
    let primary_branch_name =
        primary_branch_name(&repo).context("Determining primary branch name")?;

    let commit = if preemptive {
        if let Some(prefetch_commit) = repo
            .get_prefetch_head_commit("origin", primary_branch_name.as_str())
            .context("Resolving prefetch head commit")?
        {
            prefetch_commit
        } else {
            bail!("No prefetch commit found for preemptive sync");
        }
    } else {
        head_commit
    };

    if preemptive {
        if let Some(working_tree) = repo.working_tree() {
            if let Ok(Some(sync_point)) = working_tree.read_sparse_sync_point_ref() {
                if sync_point == commit.id() {
                    // The sync point is already set to this ref. We don't need to bother.
                    warn!("Skipping preemptive synchronization because the commit to sync is the same as that of the sync point");
                    return Ok(SyncResult {
                        checked_out: false,
                        commit_id: Some(commit.id()),
                        skipped: true,
                    });
                }
            } else if let Ok(Some(sync_point)) = working_tree.read_preemptive_sync_point_ref() {
                if sync_point == commit.id() {
                    // The sync point is already set to this ref. We don't need to bother.
                    warn!("Skipping preemptive synchronization because the commit to sync is the same as that of the preemptive sync point");
                    return Ok(SyncResult {
                        checked_out: false,
                        commit_id: Some(commit.id()),
                        skipped: true,
                    });
                }
            }
        }
    }

    let (pattern_count, checked_out) = perform("Computing the new sparse profile", || {
        let odb = RocksDBCache::new(repo.underlying());
        repo.sync(
            commit.id(),
            &targets,
            preemptive,
            &repo.config().index,
            app.clone(),
            &odb,
        )
        .context("Sync failed")
    })?;

    if preemptive {
        perform("Updating the sync point", || {
            repo.working_tree()
                .unwrap()
                .write_preemptive_sync_point_ref(commit.id())
        })?;
    } else {
        ti_client
            .get_context()
            .add_to_custom_map("pattern_count", pattern_count.to_string());
        perform("Updating the sync point", || {
            repo.working_tree().unwrap().write_sync_point_ref()
        })?;

        // The profile was successfully applied, so do not restore the backup.
        backed_up_sparse_profile.unwrap().set_restore(false);
    }

    Ok(SyncResult {
        checked_out,
        commit_id: Some(commit.id()),
        skipped: false,
    })
}

fn primary_branch_name(repo: &Repo) -> Result<String> {
    let underlying = repo.underlying();
    if underlying.find_reference("refs/heads/master").is_ok() {
        Ok(String::from("master"))
    } else if underlying.find_reference("refs/heads/main").is_ok() {
        Ok(String::from("main"))
    } else {
        bail!("Could not determine primary branch name")
    }
}
