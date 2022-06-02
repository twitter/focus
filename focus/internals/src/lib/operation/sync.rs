use crate::index::RocksDBMemoizationCacheExt;
use crate::model::repo::Repo;

use crate::operation::index;
use crate::operation::util::perform;
use crate::target::TargetSet;
use content_addressed_cache::RocksDBCache;
use focus_util::app::{App, ExitCode};
use focus_util::backed_up_file::BackedUpFile;

use std::convert::TryFrom;

use std::path::Path;

use std::sync::Arc;

use anyhow::{bail, Context, Result};

/// Synchronize the sparse repo's contents with the build graph. Returns whether a checkout actually occured.
pub fn run(sparse_repo: &Path, app: Arc<App>, fetch_index: bool) -> Result<bool> {
    let repo = Repo::open(sparse_repo, app.clone()).context("Failed to open the repo")?;
    let sparse_profile_path = repo.git_dir().join("info").join("sparse-checkout");
    if !sparse_profile_path.is_file() {
        bail!("This does not appear to be a focused repo -- it is missing a sparse checkout file");
    }
    super::ensure_clean::run(sparse_repo, app.clone())
        .context("Failed trying to determine whether working trees were clean")?;

    let backed_up_sparse_profile = BackedUpFile::new(&sparse_profile_path)?;

    let selections = repo.selection_manager()?;
    let selection = selections.computed_selection()?;
    let targets = TargetSet::try_from(&selection).context("constructing target set")?;

    // Add target/project to TI data.
    let app_for_ti_client = app.clone();
    let ti_client = app_for_ti_client.tool_insights_client();

    ti_client.get_context().add_to_custom_map("total_target_count", targets.len().to_string());
    ti_client.get_context().add_to_custom_map(
        "user_selected_project_count",
        selection.projects.len().to_string(),
    );
    ti_client.get_context().add_to_custom_map(
        "user_selected_target_count",
        selection.targets.len().to_string(),
    );

    if fetch_index {
        let _: Result<ExitCode> = index::fetch(
            app.clone(),
            index::Backend::RocksDb,
            sparse_repo.to_path_buf(),
            index::INDEX_DEFAULT_REMOTE.to_string(),
        );
    }

    let (pattern_count, checked_out) = perform("Computing the new sparse profile", || {
        let odb = RocksDBCache::new(repo.underlying());
        repo.sync(&targets, false, app.clone(), &odb)
            .context("Sync failed")
    })?;
    ti_client.get_context().add_to_custom_map("pattern_count", pattern_count.to_string());

    perform("Updating the sync point", || {
        repo.working_tree().unwrap().write_sync_point_ref()
    })?;

    // The profile was successfully applied, so do not restore the backup.
    backed_up_sparse_profile.set_restore(false);

    Ok(checked_out)
}

#[cfg(test)]
mod testing {
    use std::{collections::HashSet, path::Path};

    use anyhow::Result;
    use maplit::hashset;

    use focus_testing::init_logging;
    use focus_util::app;

    use crate::{
        model::selection::OperationAction,
        operation::{
            self,
            testing::integration::{RepoDisposition, RepoPairFixture},
        },
    };

    #[test]
    fn sync_upstream_changes() -> Result<()> {
        init_logging();

        let fixture = RepoPairFixture::new()?;

        fixture.perform_clone()?;

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
        fixture.dense_repo.write_and_commit_file(
            Path::new("x/BUILD.bazel"),
            build_bazel_content.as_bytes(),
            "Add excerpts",
        )?;
        let catz_txt_content = r#"The Naming of Cats is a difficult matter,
It isn't just one of your holiday games
        )"#;
        fixture.dense_repo.write_and_commit_file(
            Path::new("x/catz.txt"),
            catz_txt_content.as_bytes(),
            "Add excerpts",
        )?;

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
        operation::sync::run(&fixture.sparse_repo_path, fixture.app.clone(), false)?;

        let x_dir = fixture.sparse_repo_path.join("x");
        assert!(!x_dir.is_dir());

        // Add as a target
        operation::selection::add(
            &fixture.sparse_repo_path,
            false,
            vec![String::from("bazel://x/...")],
            false,
            fixture.app.clone(),
        )?;

        // Sync
        operation::sync::run(&fixture.sparse_repo_path, fixture.app.clone(), false)?;

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
            false,
            fixture.app.clone(),
        )?;
        {
            let selected_names = selected_project_names()?;
            assert_eq!(selected_names, hashset! { project_b_label.clone() })
        }
        operation::sync::run(&path, fixture.app.clone(), false)?;

        insta::assert_snapshot!(std::fs::read_to_string(&profile_path)?);
        assert!(library_b_dir.is_dir());
        assert!(project_b_dir.is_dir());

        assert!(!library_a_dir.is_dir());
        assert!(!project_a_dir.is_dir());
        operation::selection::add(
            &path,
            false,
            vec![project_a_label.clone()],
            false,
            fixture.app.clone(),
        )?;
        {
            let selected_names = selected_project_names()?;
            assert_eq!(
                selected_names,
                hashset! { project_a_label.clone(), project_b_label.clone() }
            )
        }
        operation::sync::run(&path, fixture.app.clone(), false)?;
        insta::assert_snapshot!(std::fs::read_to_string(&profile_path)?);
        assert!(library_a_dir.is_dir());
        assert!(project_a_dir.is_dir());

        operation::selection::remove(
            &path,
            false,
            vec![project_a_label],
            false,
            fixture.app.clone(),
        )?;
        {
            let selected_names = selected_project_names()?;
            assert_eq!(selected_names, hashset! { project_b_label.clone() })
        }
        operation::sync::run(&path, fixture.app.clone(), false)?;
        insta::assert_snapshot!(std::fs::read_to_string(&profile_path)?);
        assert!(!library_a_dir.is_dir());
        assert!(!project_a_dir.is_dir());

        operation::selection::remove(
            &path,
            false,
            vec![project_b_label],
            false,
            fixture.app.clone(),
        )?;
        {
            let selected_names = selected_project_names()?;
            assert_eq!(selected_names, hashset! {});
        }
        operation::sync::run(&path, fixture.app.clone(), false)?;
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
        operation::sync::run(&path, fixture.app.clone(), false)?;
        assert!(library_b_dir.is_dir());

        // operation::adhoc::pop(fixture.sparse_repo_path.clone(), 1)?;
        assert!(selections.mutate(OperationAction::Remove, &targets)?);
        selections.save()?;
        operation::sync::run(&path, fixture.app.clone(), false)?;
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
        assert!(operation::sync::run(&path, fixture.app.clone(), false)?);

        // Subsequent sync does not perform a checkout.
        assert!(!operation::sync::run(&path, fixture.app.clone(), false)?);

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
}
