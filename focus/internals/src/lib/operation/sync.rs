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
            if let Ok(Some(sync_point)) = working_tree.read_sync_point_ref() {
                if sync_point == commit.id() {
                    // The sync point is already set to this ref. We don't need to bother.
                    warn!("Skipping preemptive synchronization because the commit to sync is the same as that of the sync point");
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
            false,
            &repo.config().index,
            app.clone(),
            &odb,
        )
        .context("Sync failed")
    })?;

    if !preemptive {
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

#[cfg(test)]
mod testing {
    use std::{collections::HashSet, path::Path};

    use anyhow::Result;
    use maplit::hashset;

    use focus_testing::{init_logging, ScratchGitRepo};
    use focus_util::app;

    use crate::{
        model::{repo::Repo, selection::OperationAction},
        operation::{
            self,
            testing::integration::{RepoDisposition, RepoPairFixture},
        },
    };

    fn add_updated_content(scratch_repo: &ScratchGitRepo) -> Result<git2::Oid> {
        // Commit new files affecting the build graph to the dense repo
        let build_bazel_content = r#"filegroup(
            name = "excerpts",
            srcs = [
                "catz.txt",
            ],
            visibility = [
                "//visibility:public",
            ],
        )"#;
        scratch_repo.write_and_commit_file(
            Path::new("x/BUILD.bazel"),
            build_bazel_content.as_bytes(),
            "Add excerpts",
        )?;
        let catz_txt_content = r#"The Naming of Cats is a difficult matter,
        It isn't just one of your holiday games
                )"#;
        scratch_repo.write_and_commit_file(
            Path::new("x/catz.txt"),
            catz_txt_content.as_bytes(),
            "Add excerpts",
        )
    }

    #[test]
    fn sync_upstream_changes() -> Result<()> {
        init_logging();

        let fixture = RepoPairFixture::new()?;

        fixture.perform_clone()?;
        let _ = add_updated_content(&fixture.dense_repo)?;

        // Fetch in the sparse repo from the dense repo
        fixture.perform_pull(RepoDisposition::Sparse, "origin", "main")?;

        // Make sure that the graph is seen as having changed
        assert_eq!(
            operation::detect_build_graph_changes::run(
                &fixture.sparse_repo_path,
                vec![],
                fixture.app.clone(),
            )?,
            app::ExitCode(1)
        );

        // Sync in the sparse repo
        let _ = operation::sync::run(&fixture.sparse_repo_path, false, fixture.app.clone())?;

        let x_dir = fixture.sparse_repo_path.join("x");
        assert!(!x_dir.is_dir());

        // Add as a target
        operation::selection::add(
            &fixture.sparse_repo_path,
            false,
            vec![String::from("bazel://x/...")],
            fixture.app.clone(),
        )?;

        // Sync
        let _ = operation::sync::run(&fixture.sparse_repo_path, false, fixture.app.clone())?;

        assert!(x_dir.is_dir());

        Ok(())
    }

    #[test]
    fn sync_layer_manipulation() -> Result<()> {
        init_logging();

        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;

        let selected_project_names = || -> Result<HashSet<String>> {
            Ok(fixture
                .sparse_repo()?
                .selection_manager()?
                .computed_selection()?
                .projects
                .iter()
                .filter_map(|project| {
                    if project.mandatory {
                        None
                    } else {
                        Some(project.name.to_owned())
                    }
                })
                .collect::<HashSet<String>>())
        };

        let project_a_label = String::from("team_banzai/project_a");
        let project_b_label = String::from("team_zissou/project_b");

        let path = fixture.sparse_repo_path.clone();
        let library_a_dir = path.join("library_a");
        let project_a_dir = path.join("project_a");
        let library_b_dir = path.join("library_b");
        let project_b_dir = path.join("project_b");
        let profile_path = path.join(".git").join("info").join("sparse-checkout");

        {
            let selected_names = selected_project_names()?;
            assert_eq!(selected_names, hashset! {});
        }
        insta::assert_snapshot!(std::fs::read_to_string(&profile_path)?);

        assert!(!library_b_dir.is_dir());
        assert!(!project_b_dir.is_dir());
        operation::selection::add(
            &path,
            false,
            vec![project_b_label.clone()],
            fixture.app.clone(),
        )?;
        {
            let selected_names = selected_project_names()?;
            assert_eq!(selected_names, hashset! { project_b_label.clone() })
        }
        operation::sync::run(&path, false, fixture.app.clone())?;

        insta::assert_snapshot!(std::fs::read_to_string(&profile_path)?);
        assert!(library_b_dir.is_dir());
        assert!(project_b_dir.is_dir());

        assert!(!library_a_dir.is_dir());
        assert!(!project_a_dir.is_dir());
        operation::selection::add(
            &path,
            false,
            vec![project_a_label.clone()],
            fixture.app.clone(),
        )?;
        {
            let selected_names = selected_project_names()?;
            assert_eq!(
                selected_names,
                hashset! { project_a_label.clone(), project_b_label.clone() }
            )
        }
        operation::sync::run(&path, false, fixture.app.clone())?;
        insta::assert_snapshot!(std::fs::read_to_string(&profile_path)?);
        assert!(library_a_dir.is_dir());
        assert!(project_a_dir.is_dir());

        operation::selection::remove(&path, false, vec![project_a_label], fixture.app.clone())?;
        {
            let selected_names = selected_project_names()?;
            assert_eq!(selected_names, hashset! { project_b_label.clone() })
        }
        operation::sync::run(&path, false, fixture.app.clone())?;
        insta::assert_snapshot!(std::fs::read_to_string(&profile_path)?);
        assert!(!library_a_dir.is_dir());
        assert!(!project_a_dir.is_dir());

        operation::selection::remove(&path, false, vec![project_b_label], fixture.app.clone())?;
        {
            let selected_names = selected_project_names()?;
            assert_eq!(selected_names, hashset! {});
        }
        operation::sync::run(&path, false, fixture.app.clone())?;
        insta::assert_snapshot!(std::fs::read_to_string(&profile_path)?);

        assert!(!library_b_dir.is_dir());
        assert!(!project_b_dir.is_dir());

        Ok(())
    }

    #[test]
    fn sync_adhoc_manipulation() -> Result<()> {
        init_logging();

        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;

        let path = fixture.sparse_repo_path.clone();
        let library_b_dir = path.join("library_b");
        let targets = vec![String::from("bazel://library_b/...")];
        let mut selections = fixture.sparse_repo()?.selection_manager()?;

        assert!(selections.mutate(OperationAction::Add, &targets)?);
        selections.save()?;
        operation::sync::run(&path, false, fixture.app.clone())?;
        assert!(library_b_dir.is_dir());

        // operation::adhoc::pop(fixture.sparse_repo_path.clone(), 1)?;
        assert!(selections.mutate(OperationAction::Remove, &targets)?);
        selections.save()?;
        operation::sync::run(&path, false, fixture.app.clone())?;
        assert!(!library_b_dir.is_dir());

        Ok(())
    }

    #[test]
    fn clone_contains_top_level() -> Result<()> {
        init_logging();

        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;

        let sparse_repo = fixture.sparse_repo()?;
        let outlining_tree = sparse_repo.outlining_tree().unwrap();
        let underlying = outlining_tree.underlying();
        let outlining_tree_root = underlying.work_dir();

        let top_level_bazelisk_rc = outlining_tree_root.join(".bazeliskrc");

        let outlining_tree_git_dir = sparse_repo
            .git_dir()
            .join("worktrees")
            .join("outlining-tree");

        let profile =
            std::fs::read_to_string(outlining_tree_git_dir.join("info").join("sparse-checkout"))?;
        insta::assert_snapshot!(&profile);

        assert!(top_level_bazelisk_rc.is_file());

        Ok(())
    }

    #[test]
    fn sync_skips_checkout_with_unchanged_profile() -> Result<()> {
        init_logging();

        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;

        let path = fixture.sparse_repo_path.clone();
        let targets = vec![String::from("bazel://library_b/...")];
        let mut selections = fixture.sparse_repo()?.selection_manager()?;

        assert!(selections.mutate(OperationAction::Add, &targets)?);
        selections.save()?;
        {
            let result = operation::sync::run(&path, false, fixture.app.clone())?;
            assert!(!result.skipped);
            assert!(result.checked_out);
        }

        // Subsequent sync does not perform a checkout.
        {
            let result = operation::sync::run(&path, false, fixture.app.clone())?;
            assert!(!result.skipped);
            assert!(!result.checked_out);
        }

        Ok(())
    }

    #[test]
    fn sync_configures_working_and_outlining_trees() -> Result<()> {
        init_logging();

        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;

        // Check working tree
        let working_tree_config = fixture.sparse_repo()?.underlying().config()?.snapshot()?;
        assert!(working_tree_config.get_bool("index.sparse")?);
        assert!(working_tree_config.get_bool("core.untrackedCache")?);

        // Check outlining tree
        let outlining_tree_config = fixture
            .sparse_repo()?
            .outlining_tree()
            .unwrap()
            .underlying()
            .git_repo()
            .config()?
            .snapshot()?;
        assert!(outlining_tree_config.get_bool("index.sparse")?);
        assert!(outlining_tree_config.get_bool("core.untrackedCache")?);

        Ok(())
    }

    struct PreemptiveSyncFixture {
        pub underlying: RepoPairFixture,
        pub repo: Repo,
        pub commit_id: git2::Oid,
    }

    impl PreemptiveSyncFixture {
        fn new() -> Result<Self> {
            let fixture = RepoPairFixture::new()?;

            fixture.perform_clone()?;
            add_updated_content(&fixture.dense_repo)?;

            let fetched_commits = fixture.perform_fetch(RepoDisposition::Sparse, "origin")?;
            assert_eq!(fetched_commits.len(), 1);
            let commit_id = fetched_commits[0];

            let repo = Repo::open(&fixture.sparse_repo_path, fixture.app.clone())?;
            repo.set_preemptive_sync_enabled(true)?;

            // Set the prefetch ref
            repo.underlying().reference(
                "refs/prefetch/remotes/origin/main",
                commit_id,
                true,
                "Emulated prefetch ref",
            )?;

            Ok(PreemptiveSyncFixture {
                underlying: fixture,
                repo,
                commit_id,
            })
        }
    }

    #[test]
    fn preemptive_sync() -> Result<()> {
        init_logging();

        let fixture = PreemptiveSyncFixture::new()?;

        // Sync preemptively
        let result = operation::sync::run(
            &fixture.underlying.sparse_repo_path,
            true,
            fixture.underlying.app.clone(),
        )?;
        assert!(!result.checked_out);
        assert!(!result.skipped);

        assert_eq!(result.commit_id.unwrap(), fixture.commit_id);

        Ok(())
    }

    #[test]
    fn preemptive_sync_skips_if_disabled() -> Result<()> {
        init_logging();

        let fixture = PreemptiveSyncFixture::new()?;

        fixture.repo.set_preemptive_sync_enabled(false)?;
        let result = operation::sync::run(
            &fixture.underlying.sparse_repo_path,
            true,
            fixture.underlying.app.clone(),
        )?;
        assert!(result.skipped);

        Ok(())
    }

    // Test for already being on the commit.
}
