use std::{collections::HashSet, path::Path};

use anyhow::Result;
use maplit::hashset;

use focus_testing::{init_logging, ScratchGitRepo};
use focus_util::app;

use crate::{
    model::repo::Repo,
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
        true,
        vec![String::from("bazel://x/...")],
        fixture.app.clone(),
    )?;

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
        true,
        vec![project_b_label.clone()],
        fixture.app.clone(),
    )?;
    {
        let selected_names = selected_project_names()?;
        assert_eq!(selected_names, hashset! { project_b_label.clone() })
    }

    insta::assert_snapshot!(std::fs::read_to_string(&profile_path)?);
    assert!(library_b_dir.is_dir());
    assert!(project_b_dir.is_dir());

    assert!(!library_a_dir.is_dir());
    assert!(!project_a_dir.is_dir());
    operation::selection::add(
        &path,
        true,
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
    insta::assert_snapshot!(std::fs::read_to_string(&profile_path)?);
    assert!(library_a_dir.is_dir());
    assert!(project_a_dir.is_dir());

    operation::selection::remove(&path, true, vec![project_a_label], fixture.app.clone())?;
    {
        let selected_names = selected_project_names()?;
        assert_eq!(selected_names, hashset! { project_b_label.clone() })
    }
    insta::assert_snapshot!(std::fs::read_to_string(&profile_path)?);
    assert!(!library_a_dir.is_dir());
    assert!(!project_a_dir.is_dir());

    operation::selection::remove(&path, true, vec![project_b_label], fixture.app.clone())?;
    {
        let selected_names = selected_project_names()?;
        assert_eq!(selected_names, hashset! {});
    }
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

    operation::selection::add(
        &fixture.sparse_repo_path,
        true,
        targets.clone(),
        fixture.app.clone(),
    )?;
    assert!(library_b_dir.is_dir());

    operation::selection::remove(
        &fixture.sparse_repo_path,
        true,
        targets.clone(),
        fixture.app.clone(),
    )?;
    assert!(!library_b_dir.is_dir());

    Ok(())
}

#[test]
fn failed_selection_mutations_are_reverted() -> Result<()> {
    init_logging();

    let fixture = RepoPairFixture::new()?;
    fixture.perform_clone()?;

    let selections = fixture.sparse_repo()?.selection_manager()?;
    let selection_before = selections.selection()?;
    let targets = vec![String::from("bazel://library_z/...")];
    assert!(operation::selection::add(
        &fixture.sparse_repo_path,
        true,
        targets,
        fixture.app.clone()
    )
    .is_err());
    let selection_after = selections.selection()?;
    assert_eq!(selection_before, selection_after);

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
    operation::selection::add(&path, false, targets, fixture.app.clone())?;
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
fn preemptive_sync_skips_if_presync_ref_is_at_commit() -> Result<()> {
    init_logging();

    let fixture = PreemptiveSyncFixture::new()?;

    // Sync preemptively
    let result = operation::sync::run(
        &fixture.underlying.sparse_repo_path,
        true,
        fixture.underlying.app.clone(),
    )?;
    assert!(!result.skipped);
    assert_eq!(result.commit_id.unwrap(), fixture.commit_id);

    // Subsequent preemptive syncs are skipped
    let result = operation::sync::run(
        &fixture.underlying.sparse_repo_path,
        true,
        fixture.underlying.app.clone(),
    )?;
    assert!(result.skipped);
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
