// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::event;
use crate::init::{run_clone, CloneBuilder};
use focus_internals::index::RocksDBMemoizationCacheExt;
use focus_internals::model::selection::{Operation, OperationAction};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use content_addressed_cache::RocksDBCache;
use focus_internals::{model::repo::Repo, target::TargetSet, tracker::Tracker};

use focus_util::{self, app::App, git_helper, sandbox_command::SandboxCommandOutput};
use git2::Repository;

use std::{
    ffi::OsString,
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::{debug, error, info, info_span, warn};
use url::Url;

#[derive(Debug)]
pub enum Origin {
    /// Clone from a local path
    Local(PathBuf),

    /// Clone from a remote URL
    Remote(Url),
}

impl TryFrom<&str> for Origin {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if let Ok(url) = Url::parse(value) {
            Ok(Origin::Remote(url))
        } else {
            let dense_repo_path = PathBuf::from(value);
            let dense_repo_path = focus_util::paths::expand_tilde(dense_repo_path.as_path())?;
            Ok(Origin::Local(dense_repo_path))
        }
    }
}

#[derive(Debug)]
pub struct CloneArgs {
    pub origin: Option<Origin>,
    pub branch: String,
    pub projects_and_targets: Vec<String>,
    pub copy_branches: bool,
    pub days_of_history: u64,
    pub do_post_clone_fetch: bool,
}

impl Default for CloneArgs {
    fn default() -> CloneArgs {
        Self {
            origin: None,
            branch: String::from("master"),
            projects_and_targets: Vec::default(),
            copy_branches: true,
            days_of_history: 90,
            do_post_clone_fetch: true,
        }
    }
}

/// Entrypoint for clone operations.
#[tracing::instrument]
pub fn run(
    sparse_repo_path: PathBuf,
    clone_args: CloneArgs,
    tracker: &Tracker,
    app: Arc<App>,
) -> Result<()> {
    let CloneArgs {
        origin,
        branch,
        projects_and_targets,
        copy_branches,
        days_of_history,
        do_post_clone_fetch,
    } = clone_args;

    let origin = match origin {
        Some(origin) => origin,
        None => bail!("Clone does not have a valid origin"),
    };

    if sparse_repo_path.is_dir() {
        bail!("{} already exists", sparse_repo_path.display());
    }

    let mut tmp_sparse_repo_path = PathBuf::from(app.sandbox().path());
    tmp_sparse_repo_path.push("tmp_sparse_repo");

    let configure_repo_then_move_in_place = || -> Result<()> {
        match origin {
            Origin::Local(dense_repo_path) => clone_local(
                &dense_repo_path,
                &tmp_sparse_repo_path,
                &branch,
                copy_branches,
                days_of_history,
                app.clone(),
            ),
            Origin::Remote(url) => clone_remote(
                url,
                &tmp_sparse_repo_path,
                &branch,
                days_of_history,
                app.clone(),
            ),
        }?;

        set_up_sparse_repo(&tmp_sparse_repo_path, projects_and_targets, app.clone())?;

        if do_post_clone_fetch {
            fetch_default_remote(&tmp_sparse_repo_path, app.clone())
                .context("Could not complete post clone fetch")?;
        }

        set_up_hooks(&tmp_sparse_repo_path)?;

        move_repo(
            &tmp_sparse_repo_path,
            &sparse_repo_path,
            tracker,
            app.clone(),
        )
        .context("Could not move repo into place")?;

        Ok(())
    };

    if let err @ Err(_) = configure_repo_then_move_in_place() {
        //cleanup, will rely on maintanence sandbox cleanup for tmp_sparse_repo
        if std::fs::remove_dir_all(sparse_repo_path).is_err() {
            return err.context("Failed to cleanup repo");
        };

        return err;
    }

    Ok(())
}

fn move_repo(from_path: &Path, to_path: &Path, tracker: &Tracker, app: Arc<App>) -> Result<()> {
    std::fs::rename(from_path, to_path)?;

    // The outlining tree needs to be updated
    let repo = Repo::open(to_path, app.clone()).context("Failed to open repo")?;
    repo.repair_outlining_tree()
        .context("Failed to repair the outlining tree after move")?;

    //register the repo
    tracker
        .ensure_registered(to_path, app)
        .context("Registering repo")?;

    Ok(())
}

/// Clone from a local path on disk.
fn clone_local(
    dense_repo_path: &Path,
    sparse_repo_path: &Path,
    branch: &str,
    copy_branches: bool,
    days_of_history: u64,
    app: Arc<App>,
) -> Result<()> {
    info!("Dense repo path: {}", dense_repo_path.display());
    let dense_repo_path = if dense_repo_path.is_absolute() {
        dense_repo_path.to_owned()
    } else {
        std::env::current_dir()
            .expect("Failed determining current directory")
            .join(dense_repo_path)
    };

    if !dense_repo_path.is_absolute() {
        bail!("Dense repo path must be absolute");
    }

    if sparse_repo_path.is_dir() {
        bail!("Sparse repo directory already exists");
    }

    enable_filtering(&dense_repo_path)
        .context("setting configuration options in the dense repo")?;

    let url = Url::from_file_path(&dense_repo_path)
        .expect("Failed to convert dense repo path to a file URL");

    {
        let span = info_span!("Cloning", dense_repo_path = ?dense_repo_path, sparse_repo_path = ?sparse_repo_path);
        let _guard = span.enter();
        clone_shallow(
            &url,
            sparse_repo_path,
            branch,
            copy_branches,
            days_of_history,
            app.clone(),
        )
        .context("Failed to clone the repository")?;
    }

    let dense_repo = Repository::open(&dense_repo_path).context("Opening dense repo")?;
    let sparse_repo = Repository::open(&sparse_repo_path).context("Opening sparse repo")?;

    if copy_branches {
        let span = info_span!("Copying branches");
        let _guard = span.enter();
        copy_local_branches(
            &dense_repo,
            &sparse_repo,
            branch,
            app.clone(),
            days_of_history,
        )
        .context("Failed to copy references")?;
    }

    set_up_remotes(&dense_repo, &sparse_repo, app).context("Failed to set up the remotes")?;

    Ok(())
}

/// Enable filtering in the dense repo
fn enable_filtering(dense_repo_path: &Path) -> Result<()> {
    let description = format!(
        "Setting options in dense repository {}",
        dense_repo_path.display()
    );
    let repo = Repository::open(dense_repo_path)
        .context(description.clone())
        .context("Opening repository")?;
    let mut config = repo
        .config()
        .context(description.clone())
        .context("Reading configuration")?;
    config
        .set_bool("uploadPack.allowFilter", true)
        .context(description)
        .context("Writing configuration")?;
    Ok(())
}

fn clone_remote(
    dense_repo_url: Url,
    sparse_repo_path: &Path,
    branch: &str,
    days_of_history: u64,
    app: Arc<App>,
) -> Result<()> {
    if sparse_repo_path.is_dir() {
        bail!("Sparse repo directory already exists");
    }

    info!(
        "Cloning {} to {}",
        &dense_repo_url,
        &sparse_repo_path.display()
    );

    // Clone the repository.
    clone_shallow(
        &dense_repo_url,
        sparse_repo_path,
        branch,
        false,
        days_of_history,
        app,
    )
    .context("Failed to clone the repository")
}

fn set_up_sparse_repo(
    sparse_repo_path: &Path,
    projects_and_targets: Vec<String>,
    app: Arc<App>,
) -> Result<()> {
    {
        let repo = Repo::open(sparse_repo_path, app.clone()).context("Failed to open repo")?;
        // TODO: Parallelize these tree set up processes.
        info!("Setting up the outlining tree");
        repo.create_outlining_tree()
            .context("Failed to create the outlining tree")?;

        info!("Setting up the working tree");
        repo.create_working_tree()
            .context("Failed to create the working tree")?;
    }

    // N.B. we must re-open the repo because otherwise it has no trees...
    let repo = Repo::open(sparse_repo_path, app.clone()).context("Failed to open repo")?;
    let head_commit = repo.get_head_commit().context("Resolving head commit")?;
    let target_set = compute_and_store_initial_selection(&repo, projects_and_targets)?;

    let odb = RocksDBCache::new(repo.underlying());
    repo.sync(
        head_commit.id(),
        &target_set,
        false,
        &repo.config().index,
        app,
        &odb,
    )
    .context("Sync failed")?;

    repo.working_tree().unwrap().write_sync_point_ref()?;

    info!("Writing git config to support instrumentation");
    repo.write_git_config_to_support_instrumentation()
        .context("Could not write git config to support instrumentation")?;

    set_up_bazel_preflight_script(sparse_repo_path)?;

    Ok(())
}

fn compute_and_store_initial_selection(
    repo: &Repo,
    projects_and_targets: Vec<String>,
) -> Result<TargetSet> {
    let mut selections = repo.selection_manager()?;
    let operations = projects_and_targets
        .iter()
        .map(|value| Operation::new(OperationAction::Add, value))
        .collect::<Vec<Operation>>();
    let result = selections.process(&operations)?;
    if !result.is_success() {
        bail!("Selecting projects and targets failed");
    }
    selections.save()?;
    let selection = selections.computed_selection()?;
    let target_set = TargetSet::try_from(&selection)?;

    Ok(target_set)
}

fn clone_shallow(
    source_url: &Url,
    destination_path: &Path,
    branch: &str,
    copy_branches: bool,
    days_of_history: u64,
    app: Arc<App>,
) -> Result<()> {
    // Unfortunately time::duration is signed
    let days_of_history: i64 = days_of_history.try_into()?;

    let shallow_since_datestamp =
        focus_util::time::formatted_datestamp_at_day_in_past(days_of_history)?;

    let mut builder = CloneBuilder::new(destination_path.into());
    builder
        .fetch_url(source_url.as_str().into())
        .no_checkout(true)
        .follow_tags(false)
        .branch(branch.into())
        .add_clone_arg(format!("--shallow-since={}", shallow_since_datestamp));
    if !copy_branches {
        builder.add_clone_arg("--single-branch");
    }
    run_clone(builder, app)?;
    Ok(())
}

fn set_up_remotes(dense_repo: &Repository, sparse_repo: &Repository, app: Arc<App>) -> Result<()> {
    let remotes = dense_repo
        .remotes()
        .context("Failed to read remotes from dense repo")?;

    let sparse_workdir = sparse_repo
        .workdir()
        .expect("Could not determine sparse repo workdir");

    for remote_name in remotes.iter() {
        let remote_name = match remote_name {
            Some(name) => name,
            None => continue,
        };

        let dense_remote = dense_repo.find_remote(remote_name)?;
        let url = if let Some(url) = dense_remote.url() {
            url
        } else {
            bail!("Dense remote '{}' has no URL", remote_name);
        };

        let push_url = dense_remote.pushurl().unwrap_or(url);
        debug!(
            "Setting up remote {} (fetch={}, push={})",
            remote_name, url, push_url
        );

        let mut fetch_url = Url::parse(url).with_context(|| {
            format!(
                "Failed to parse the URL ('{}') for remote {}",
                url, remote_name
            )
        })?;
        let push_url = Url::parse(push_url).with_context(|| {
            format!(
                "Failed to parse the push URL ('{}') for remote {}",
                fetch_url, remote_name
            )
        })?;

        if let Some(host) = fetch_url.host() {
            if cfg!(feature = "twttr") {
                // Apply Twitter-specific remote treatment.
                if host.to_string().eq_ignore_ascii_case("git.twitter.biz") {
                    // If the path for the fetch URL does not begin with '/ro', add that prefix.
                    if !fetch_url.path().starts_with("/ro") {
                        fetch_url.set_path(&format!("/ro{}", fetch_url.path()));
                    }
                }
            }
        } else {
            bail!("Fetch URL for remote '{}' has no host", remote_name);
        }

        // Delete existing remote in the sparse repo if it exists. This is a workaround because `remote_delete` is not working correctly.
        if sparse_repo.find_remote(remote_name).is_ok() {
            let (mut cmd, scmd) = git_helper::git_command(app.clone())?;
            let _ = scmd.ensure_success_or_log(
                cmd.current_dir(sparse_workdir)
                    .arg("remote")
                    .arg("remove")
                    .arg(remote_name),
                SandboxCommandOutput::Stderr,
            )?;
        }

        // Add the remote to the sparse repo
        info!(
            "Setting up remote {} fetch:{} push:{}",
            remote_name,
            fetch_url.as_str(),
            push_url.as_str()
        );
        sparse_repo
            .remote_with_fetch(
                remote_name,
                fetch_url.as_str(),
                format!("refs/heads/master:refs/remotes/{}/master", remote_name).as_str(),
            )
            .with_context(|| {
                format!(
                    "Configuring fetch URL remote {} in the sparse repo failed",
                    &remote_name
                )
            })?;

        sparse_repo
            .config()?
            .set_str(
                format!("remote.{}.tagOpt", remote_name).as_str(),
                "--no-tags",
            )
            .with_context(|| format!("setting remote.{}.tagOpt = --no-tags", &remote_name))?;

        sparse_repo
            .remote_set_pushurl(remote_name, Some(push_url.as_str()))
            .with_context(|| {
                format!(
                    "Configuring push URL for remote {} in the sparse repo failed",
                    &remote_name
                )
            })?;
    }

    Ok(())
}

fn set_up_hooks(sparse_repo: &Path) -> Result<()> {
    event::init(sparse_repo)?;
    Ok(())
}

/// Issues a git command to fetch from the default remote.
///
/// Uses a git command instead of using git2 since git2 does not seem to read from the correct config on fetch.
fn fetch_default_remote(sparse_repo: &Path, app: Arc<App>) -> Result<()> {
    let (mut cmd, scmd) = git_helper::git_command(app)?;
    let _ = scmd.ensure_success_or_log(
        cmd.current_dir(&sparse_repo).arg("fetch").arg("origin"),
        SandboxCommandOutput::Stderr,
    )?;

    Ok(())
}

fn copy_local_branches(
    dense_repo: &Repository,
    sparse_repo: &Repository,
    branch: &str,
    app: Arc<App>,
    days_of_history: u64,
) -> Result<()> {
    let branches = dense_repo
        .branches(Some(git2::BranchType::Local))
        .context("Failed to enumerate local branches in the dense repo")?;
    let mut valid_local_branches = Vec::new();

    for b in branches {
        let (b, _branch_type) = b?;
        let name = match b.name()? {
            Some(name) => name,
            None => {
                warn!(
                    "Skipping branch {:?} because it is not representable as UTF-8",
                    b.name_bytes()
                );
                continue;
            }
        };

        if name == branch {
            // Skip the primary branch since it should already be configured.
            continue;
        }

        debug!("Examining dense repo branch {}", name);
        let dense_commit = b
            .get()
            .peel_to_commit()
            .context("Failed to peel branch ref to commit")?;

        let dense_commit_date = DateTime::from_utc(
            NaiveDateTime::from_timestamp(dense_commit.time().seconds(), 0),
            Utc,
        )
        .date();

        let days_of_history: i64 = days_of_history.try_into()?;
        let shallow_since_datestamp = focus_util::time::date_at_day_in_past(days_of_history)?;

        if dense_commit_date > shallow_since_datestamp {
            valid_local_branches.push((name.to_owned(), dense_commit.to_owned()));
        } else {
            warn!(
                "Branch {} is older than the configured limit ({}). Rebase it if you would like it to be included in the new repo.",
                name, shallow_since_datestamp
            );
        }
    }

    let branch_list_output = valid_local_branches
        .iter()
        .map(|(name, _)| name.to_string())
        .collect::<Vec<String>>()
        .join(" ");

    let (mut cmd, scmd) = git_helper::git_command(app)?;
    let mut args: Vec<OsString> = vec!["fetch".into(), "--no-tags".into()];
    args.push("origin".into());
    valid_local_branches
        .iter()
        .for_each(|(name, _)| args.push(name.into()));
    scmd.ensure_success_or_log(
        cmd.current_dir(sparse_repo.path()).args(args),
        SandboxCommandOutput::Stderr,
    )
    .map(|_| ())
    .with_context(|| {
        format!(
            "Failed to fetch user branches ({}) for {}",
            branch_list_output,
            whoami::username()
        )
    })?;

    valid_local_branches.iter().for_each(|(name, dense_commit)| {
        match sparse_repo.find_commit(dense_commit.id()) {
            Ok(sparse_commit) => match sparse_repo.branch(name, &sparse_commit, false) {
                Ok(_new_branch) => {
                    info!("Created branch {} ({})", name, sparse_commit.id());
                }
                Err(e) => {
                    error!("Could not create branch {} in the sparse repo: {}", name, e);
                }
            },
            Err(_) => {
                error!("Could not create branch {} in the sparse repo because the associated commit ({}) does not exist!",
                    name, dense_commit.id());
            }
        }
    });

    Ok(())
}

// Set git config key focus.sync-point to HEAD
fn set_up_bazel_preflight_script(sparse_repo: &Path) -> Result<()> {
    use std::io::prelude::*;
    use std::os::unix::prelude::PermissionsExt;

    let sparse_focus_dir = sparse_repo.join(".focus");
    if !sparse_focus_dir.is_dir() {
        std::fs::create_dir(sparse_focus_dir.as_path()).with_context(|| {
            format!("failed to create directory {}", sparse_focus_dir.display())
        })?;
    }
    let preflight_script_path = sparse_focus_dir.join("preflight");
    {
        let mut preflight_script_file = BufWriter::new(
            File::create(preflight_script_path.as_path())
                .context("writing the build preflight script")?,
        );

        writeln!(preflight_script_file, "#!/bin/sh")?;
        writeln!(preflight_script_file)?;
        writeln!(
            preflight_script_file,
            "RUST_LOG=error exec focus detect-build-graph-changes"
        )?;
    }

    let mut perms = std::fs::metadata(preflight_script_path.as_path())
        .context("Reading permissions of the preflight script failed")?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(preflight_script_path, perms)
        .context("Setting permissions of the preflight script failed")?;

    Ok(())
}

#[cfg(test)]
mod test {
    use crate::testing::integration::RepoPairFixture;
    use focus_internals::target::Target;
    use focus_testing::init_logging;

    use anyhow::Result;

    #[test]
    fn clone_contains_an_initial_layer_set() -> Result<()> {
        init_logging();

        let mut fixture = RepoPairFixture::new()?;
        let library_a_target_expression = String::from("bazel://library_a/...");
        let project_b_label = String::from("team_zissou/project_b");
        fixture
            .projects_and_targets
            .push(library_a_target_expression.clone());
        fixture.projects_and_targets.push(project_b_label.clone());

        fixture.perform_clone()?;

        let selections = fixture.sparse_repo()?.selection_manager()?;
        let selection = selections.computed_selection()?;

        let library_a_target = Target::try_from(library_a_target_expression.as_str())?;
        let project_b = selections
            .project_catalog()
            .optional_projects
            .underlying
            .get(&project_b_label)
            .unwrap();

        assert!(selection.targets.contains(&library_a_target));
        assert!(selection.projects.contains(project_b));

        Ok(())
    }
}

#[cfg(feature = "twttr")]
#[cfg(test)]
mod twttr_test {
    use std::{
        collections::HashSet,
        path::{Path, PathBuf},
        sync::Arc,
    };

    use crate::testing::integration::RepoPairFixture;
    use anyhow::{bail, Context, Result};
    use assert_cmd::prelude::OutputAssertExt;
    use focus_internals::model::repo::Repo;
    use focus_testing::init_logging;
    use focus_util::app::App;
    use git2::Repository;
    use tracing::info;

    const MAIN_BRANCH_NAME: &str = "main";

    #[cfg(feature = "twttr")]
    #[test]
    fn local_clone_smoke_test() -> Result<()> {
        init_logging();
        let fixture = RepoPairFixture::new()?;

        // Set up a remote that mimics source so that we can check that the setting of fetch and push URLs.
        fixture
            .app
            .git_binary()
            .command()
            .arg("remote")
            .arg("add")
            .arg("origin")
            .arg("https://git.twitter.biz/focus-test-repo")
            .current_dir(&fixture.dense_repo_path)
            .assert()
            .success();

        // Make a branch that shouldn't end up in the sparse repo
        fixture
            .dense_repo
            .create_and_switch_to_branch("old_branch")?;
        fixture.dense_repo.make_empty_commit(
            "I'm too old to be in the sparse repo!",
            Some("Sun, Jan 27 22:32:18 2008"),
        )?;

        // Make a branch that should end up in the sparse repo.
        fixture
            .dense_repo
            .create_and_switch_to_branch("branch_two")?;
        fixture
            .dense_repo
            .make_empty_commit("I'm fresh and I should be in the sparse repo!", None)?;

        let app = Arc::new(App::new_for_testing()?);

        fixture.perform_clone().context("Clone failed")?;

        let git_repo = Repository::open(&fixture.sparse_repo_path)?;
        assert!(!git_repo.is_bare());

        // Check `focus.version` gets set
        assert_eq!(
            git_repo.config()?.snapshot()?.get_str("focus.version")?,
            env!("CARGO_PKG_VERSION")
        );

        // Check `twitter.statsenabled` gets set
        assert!(git_repo
            .config()?
            .snapshot()?
            .get_bool("twitter.statsenabled")?);

        // Check `ci.alt.enabled` gets set
        assert!(git_repo.config()?.snapshot()?.get_bool("ci.alt.enabled")?);

        // Check `ci.alt.remote` gets set
        assert_eq!(
            git_repo.config()?.snapshot()?.get_str("ci.alt.remote")?,
            "https://git.twitter.biz/source-ci"
        );

        // Check the remote URLs
        let origin_remote = git_repo.find_remote("origin")?;
        assert_eq!(
            origin_remote.url().unwrap(),
            "https://git.twitter.biz/ro/focus-test-repo"
        );
        assert_eq!(
            origin_remote.pushurl().unwrap(),
            "https://git.twitter.biz/focus-test-repo"
        );

        // Check branches
        let main_branch = git_repo
            .find_branch(MAIN_BRANCH_NAME, git2::BranchType::Local)
            .context("Failed to find main branch")?;
        let main_branch_commit_id = main_branch.get().peel_to_commit()?.id();

        for possible_branch in git_repo.branches(None)? {
            match possible_branch {
                Ok((branch, kind)) => {
                    info!("{:?} branch: {}", kind, branch.name().unwrap().unwrap());
                }
                Err(e) => {
                    bail!("Error enumerating local branches: {}", e);
                }
            }
        }

        git_repo
            .find_branch("branch_two", git2::BranchType::Local)
            .context("Failed to find branch_two")?;

        assert!(
            git_repo
                .find_branch("old_branch", git2::BranchType::Local)
                .is_err(),
            "old_branch was copied to sparse repo, despite being too old for the shallow window"
        );

        // Check post-merge hook
        let focus_exe = &std::env::current_exe().unwrap_or_else(|_| PathBuf::from("focus"));
        let focus_exe_path = focus_exe.file_name().unwrap().to_string_lossy();
        let post_merge_hook_contents =
            std::fs::read_to_string(git_repo.path().join("hooks").join("post-merge"))
                .expect("Something went wrong reading the file");
        assert_eq!(
            post_merge_hook_contents,
            format!("{} event post-merge\n", focus_exe_path)
        );

        // TODO: Test refspecs from remote config
        let model_repo = Repo::open(&fixture.sparse_repo_path, app)?;

        // Check sync point
        let sync_point_oid = model_repo
            .working_tree()
            .unwrap()
            .read_sparse_sync_point_ref()?
            .unwrap();
        assert_eq!(sync_point_oid, main_branch_commit_id);

        // Check tree contents
        {
            let outlining_tree = model_repo.outlining_tree().unwrap();
            let outlining_tree_underlying = outlining_tree.underlying();
            let outlining_tree_path = outlining_tree_underlying.work_dir();
            let walker = walkdir::WalkDir::new(outlining_tree_path).follow_links(false);

            let outlining_tree_paths: HashSet<PathBuf> = walker
                .into_iter()
                .map(|m| {
                    m.unwrap()
                        .path()
                        .strip_prefix(&outlining_tree_path)
                        .unwrap()
                        .to_owned()
                })
                .collect();

            assert!(outlining_tree_paths.contains(Path::new("focus")));
            assert!(outlining_tree_paths.contains(Path::new("WORKSPACE")));
            assert!(outlining_tree_paths.contains(Path::new("library_a/BUILD")));
            assert!(outlining_tree_paths.contains(Path::new("library_b/BUILD")));
            assert!(outlining_tree_paths.contains(Path::new(
                "project_a/src/main/java/com/example/cmdline/BUILD"
            )));
            assert!(outlining_tree_paths.contains(Path::new(
                "project_b/src/main/java/com/example/cmdline/BUILD"
            )));
            assert!(outlining_tree_paths.contains(Path::new("mandatory_z/BUILD")));
        }

        {
            let working_tree = model_repo.working_tree().unwrap();
            let working_tree_path = working_tree.work_dir();
            let walker = walkdir::WalkDir::new(working_tree_path).follow_links(false);
            let working_tree_paths: HashSet<PathBuf> = walker
                .into_iter()
                .map(|m| {
                    m.unwrap()
                        .path()
                        .strip_prefix(&working_tree_path)
                        .unwrap()
                        .to_owned()
                })
                .collect();

            // N.B. Only the mandatory project is checked out
            assert!(working_tree_paths.contains(Path::new("focus")));
            assert!(working_tree_paths.contains(Path::new("mandatory_z")));
            assert!(working_tree_paths.contains(Path::new("mandatory_z/BUILD")));
            assert!(working_tree_paths.contains(Path::new("mandatory_z/quotes.txt")));

            assert!(!working_tree_paths.contains(Path::new("library_a/BUILD")));
            assert!(!working_tree_paths.contains(Path::new("library_b/BUILD")));
            assert!(!working_tree_paths.contains(Path::new("library_a/BUILD")));
            assert!(!working_tree_paths.contains(Path::new("library_a/BUILD")));
            assert!(!working_tree_paths.contains(Path::new(
                "project_a/src/main/java/com/example/cmdline/BUILD"
            )));
            assert!(!working_tree_paths.contains(Path::new(
                "project_b/src/main/java/com/example/cmdline/BUILD"
            )));
        }

        Ok(())
    }
}
