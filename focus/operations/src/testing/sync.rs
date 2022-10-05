// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use focus_internals::model::repo::Repo;
use focus_testing::ScratchGitRepo;
use insta::assert_snapshot;
use std::{collections::HashSet, path::Path, time::Duration};

use anyhow::Result;
use maplit::hashset;

use focus_testing::init_logging;
use focus_util::app;

use crate::{
    sync::{SyncMechanism, SyncMode, SyncStatus},
    testing::integration::{RepoDisposition, RepoPairFixture},
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
        crate::detect_build_graph_changes::run(
            &fixture.sparse_repo_path,
            false,
            vec![],
            fixture.app.clone(),
        )?,
        app::ExitCode(1)
    );

    // Sync in the sparse repo
    let sync_result = crate::sync::run(
        &fixture.sparse_repo_path,
        SyncMode::Normal,
        fixture.app.clone(),
    )?;
    assert_eq!(sync_result.mechanism, SyncMechanism::Outline);

    let x_dir = fixture.sparse_repo_path.join("x");
    assert!(!x_dir.is_dir());

    // Add as a target
    crate::selection::add(
        &fixture.sparse_repo_path,
        true,
        vec![String::from("bazel://x/...")],
        false,
        fixture.app.clone(),
    )?;

    assert!(x_dir.is_dir());

    Ok(())
}

#[test]
fn sync_detect_graph_changes_advisory() -> Result<()> {
    init_logging();

    let fixture = RepoPairFixture::new()?;

    fixture.perform_clone()?;

    let _ = add_updated_content(&fixture.dense_repo)?;

    // Fetch in the sparse repo from the dense repo
    fixture.perform_pull(RepoDisposition::Sparse, "origin", "main")?;

    // In advisory mode, detect_build_graph_changes exits successfully
    assert_eq!(
        crate::detect_build_graph_changes::run(
            &fixture.sparse_repo_path,
            true,
            vec![],
            fixture.app.clone(),
        )?,
        app::ExitCode(0)
    );

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
    crate::selection::add(
        &path,
        true,
        vec![project_b_label.clone()],
        false,
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
    crate::selection::add(
        &path,
        true,
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
    insta::assert_snapshot!(std::fs::read_to_string(&profile_path)?);
    assert!(library_a_dir.is_dir());
    assert!(project_a_dir.is_dir());

    crate::selection::remove(&path, true, vec![project_a_label], fixture.app.clone())?;
    {
        let selected_names = selected_project_names()?;
        assert_eq!(selected_names, hashset! { project_b_label.clone() })
    }
    insta::assert_snapshot!(std::fs::read_to_string(&profile_path)?);
    assert!(!library_a_dir.is_dir());
    assert!(!project_a_dir.is_dir());

    crate::selection::remove(&path, true, vec![project_b_label], fixture.app.clone())?;
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

    crate::selection::add(
        &fixture.sparse_repo_path,
        true,
        targets.clone(),
        false,
        fixture.app.clone(),
    )?;
    assert!(library_b_dir.is_dir());

    crate::selection::remove(
        &fixture.sparse_repo_path,
        true,
        targets,
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
    assert!(crate::selection::add(
        &fixture.sparse_repo_path,
        true,
        targets,
        false,
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
    let profile_path = fixture
        .sparse_repo()
        .unwrap()
        .working_tree()
        .unwrap()
        .sparse_checkout_path();

    let targets = vec![String::from("bazel://library_b/...")];
    crate::selection::add(
        &fixture.sparse_repo_path,
        false, // Note: Manual sync
        targets,
        false,
        fixture.app.clone(),
    )?;
    // First sync performs a checkout.
    assert!(crate::sync::run(&path, SyncMode::Normal, fixture.app.clone())?.checked_out);
    let original_profile_contents = std::fs::read_to_string(&profile_path)?;
    assert_snapshot!(original_profile_contents);

    // Subsequent sync does not perform a checkout.
    let sync_result = crate::sync::run(&path, SyncMode::Normal, fixture.app.clone())?;
    let updated_profile_contents = std::fs::read_to_string(&profile_path)?;
    assert_snapshot!(original_profile_contents);
    assert_eq!(&original_profile_contents, &updated_profile_contents);
    assert!(!sync_result.checked_out);
    assert_eq!(sync_result.mechanism, SyncMechanism::Outline);

    Ok(())
}

#[cfg(feature = "twttr")]
#[test]
fn sync_sets_ti_client_correctly() -> Result<()> {
    init_logging();

    let fixture = RepoPairFixture::new()?;
    fixture.perform_clone()?;

    let project_a_label = String::from("team_banzai/project_a");
    let targets = vec![project_a_label, String::from("bazel://library_b/...")];

    crate::selection::add(
        &fixture.sparse_repo_path,
        true,
        targets.clone(),
        false,
        fixture.app.clone(),
    )?;

    // Get the actual strings set in TI client by the sync above
    let reported_projects = fixture
        .app
        .tool_insights_client()
        .get_inner()
        .get_ti_context()
        .get_custom_map()
        .unwrap()
        .get("user_project_selection")
        .unwrap()
        .clone();
    let reported_targets = fixture
        .app
        .tool_insights_client()
        .get_inner()
        .get_ti_context()
        .get_custom_map()
        .unwrap()
        .get("user_target_selection")
        .unwrap()
        .clone();

    // Calculate the expected strings from the selection of the repo.
    let selections = fixture
        .sparse_repo()
        .unwrap()
        .selection_manager()
        .unwrap()
        .computed_selection()
        .unwrap();
    let mut project_selection_names: Vec<String> =
        selections.projects.iter().map(|n| n.name.clone()).collect();
    let mut target_selection_names: Vec<String> =
        selections.targets.iter().map(|n| n.to_string()).collect();
    project_selection_names.sort();
    target_selection_names.sort();
    let expected_projects_string = serde_json::to_string(&project_selection_names).unwrap();
    let expected_target_string = serde_json::to_string(&target_selection_names).unwrap();

    // First assert that expected hasn't changed
    assert_eq!(
        expected_projects_string,
        "[\"mandatory\",\"team_banzai/project_a\"]"
    );
    assert_eq!(expected_target_string, "[\"bazel://library_b/...\"]");

    // Assert that expected strings from TI match the ones in TI client
    assert_eq!(expected_projects_string, reported_projects);
    assert_eq!(expected_target_string, reported_targets);
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

#[cfg(not(feature = "ci"))]
#[test]
fn regression_adding_directory_targets_present_in_mandatory_sets() -> Result<()> {
    init_logging();

    let fixture = RepoPairFixture::new()?;
    fixture.perform_clone()?;

    let path = fixture.sparse_repo_path.clone();

    let mandatory_y_dir = path.join("mandatory_y");
    let very_important_info_dir = mandatory_y_dir.join("very_important_info");
    let automakers_dir = very_important_info_dir.join("automakers");
    let swedish_txt_file = automakers_dir.join("swedish.txt");
    let targets = vec![String::from("directory:mandatory_y")];
    assert!(automakers_dir.is_dir());
    assert!(swedish_txt_file.is_file());
    crate::selection::add(
        &fixture.sparse_repo_path,
        true,
        targets,
        false,
        fixture.app.clone(),
    )?;
    assert!(swedish_txt_file.is_file());

    Ok(())
}

#[test]
fn regression_adding_deep_directory_target_materializes_correctly() -> Result<()> {
    init_logging();

    let fixture = RepoPairFixture::new()?;
    fixture.perform_clone()?;

    let path = fixture.sparse_repo_path.clone();

    let w_dir = path.join("w_dir");
    let x_dir = w_dir.join("x_dir");
    let y_dir = x_dir.join("y_dir");
    let z_dir = y_dir.join("z_dir");
    let w_build_file = w_dir.join("BUILD.bazel");
    let x_text_file = x_dir.join("x.txt");
    let z_text_file = z_dir.join("z.txt");
    let targets = vec![String::from("directory:w_dir/x_dir/y_dir/z_dir")];

    // None of these should exist since they aren't mandatory.
    assert!(!z_dir.is_dir());
    assert!(!w_build_file.is_file());
    assert!(!x_text_file.is_file());
    assert!(!z_text_file.is_file());

    crate::selection::add(
        &fixture.sparse_repo_path,
        true,
        targets,
        false,
        fixture.app.clone(),
    )?;

    assert!(w_build_file.is_file());
    assert!(x_text_file.is_file());
    assert!(z_text_file.is_file());

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
#[ignore] // these must be run single-threaded
fn preemptive_sync_single_threaded_test() -> Result<()> {
    init_logging();

    let fixture = PreemptiveSyncFixture::new()?;

    // Sync preemptively
    fixture.repo.set_preemptive_sync_enabled(true)?;
    fixture
        .repo
        .set_preemptive_sync_idle_threshold(Duration::from_millis(150))?;
    crate::sync::test_only_set_preemptive_sync_machine_is_active(false);
    let result = crate::sync::run(
        &fixture.underlying.sparse_repo_path,
        SyncMode::Preemptive { force: false },
        fixture.underlying.app.clone(),
    )?;
    assert!(!result.checked_out);
    assert_eq!(result.status, SyncStatus::Success);

    assert_eq!(result.commit_id.unwrap(), fixture.commit_id);

    Ok(())
}

#[test]
#[ignore] // these must be run single-threaded
fn preemptive_sync_skips_if_presync_ref_is_at_commit_single_threaded_test() -> Result<()> {
    init_logging();

    let fixture = PreemptiveSyncFixture::new()?;

    // Sync preemptively
    fixture
        .repo
        .set_preemptive_sync_idle_threshold(Duration::from_millis(150))?;
    crate::sync::test_only_set_preemptive_sync_machine_is_active(false);
    let result = crate::sync::run(
        &fixture.underlying.sparse_repo_path,
        SyncMode::Preemptive { force: false },
        fixture.underlying.app.clone(),
    )?;
    assert_eq!(result.status, SyncStatus::Success);
    assert_eq!(result.commit_id.unwrap(), fixture.commit_id);

    // Subsequent preemptive syncs are skipped
    let result = crate::sync::run(
        &fixture.underlying.sparse_repo_path,
        SyncMode::Preemptive { force: false },
        fixture.underlying.app.clone(),
    )?;
    assert_eq!(result.status, SyncStatus::SkippedSyncPointUnchanged);
    assert_eq!(result.commit_id.unwrap(), fixture.commit_id);

    Ok(())
}

#[test]
#[ignore] // these must be run single-threaded
fn preemptive_sync_skips_if_disabled_single_threaded_test() -> Result<()> {
    init_logging();

    let fixture = PreemptiveSyncFixture::new()?;

    fixture.repo.set_preemptive_sync_enabled(false)?;
    fixture
        .repo
        .set_preemptive_sync_idle_threshold(Duration::from_millis(150))?;
    crate::sync::test_only_set_preemptive_sync_machine_is_active(false);
    let result = crate::sync::run(
        &fixture.underlying.sparse_repo_path,
        SyncMode::Preemptive { force: false },
        fixture.underlying.app.clone(),
    )?;
    assert_eq!(result.status, SyncStatus::SkippedPreemptiveSyncDisabled);

    Ok(())
}

#[test]
#[ignore] // these must be run single-threaded
fn preemptive_sync_skips_if_machine_is_in_active_use_single_threaded_test() -> Result<()> {
    init_logging();

    let fixture = PreemptiveSyncFixture::new()?;

    fixture.repo.set_preemptive_sync_enabled(true)?;
    fixture
        .repo
        .set_preemptive_sync_idle_threshold(Duration::from_millis(150))?;
    crate::sync::test_only_set_preemptive_sync_machine_is_active(true);
    let result = crate::sync::run(
        &fixture.underlying.sparse_repo_path,
        SyncMode::Preemptive { force: false },
        fixture.underlying.app.clone(),
    )?;
    assert_eq!(
        result.status,
        SyncStatus::SkippedPreemptiveSyncCancelledByActivity
    );
    crate::sync::test_only_set_preemptive_sync_machine_is_active(false);

    Ok(())
}
