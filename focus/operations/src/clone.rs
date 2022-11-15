// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::event;
use crate::sync::SyncMode;
use focus_internals::index::RocksDBMemoizationCacheExt;
use focus_internals::model::selection::{Operation, OperationAction};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use content_addressed_cache::RocksDBCache;
use focus_internals::{model::repo::Repo, target::TargetSet, tracker::Tracker};

use focus_util::sandbox_command::SandboxCommand;
use focus_util::{self, app::App, git_helper, sandbox_command::SandboxCommandOutput};
use git2::Repository;

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::process::Command;
use std::{
    ffi::OsString,
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::{debug, error, info, info_span, warn};
use url::Url;

pub fn run_clone(mut clone_builder: CloneBuilder, app: Arc<App>) -> Result<()> {
    clone_builder.add_clone_args(vec!["--progress"]);
    let (mut cmd, scmd) = clone_builder.build(app)?;

    scmd.ensure_success_or_log(&mut cmd, SandboxCommandOutput::Stderr)
        .map(|_| ())
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub enum InitOpt {
    NoCheckout,
    FollowTags,
    Bare,
    Sparse,
    Progress,
}

#[derive(Debug)]
pub struct CloneBuilder {
    // note, to be totally typesafe we'd have a type that represented
    // when the target_path was set, and another to represent another
    // where it was unset, but that's unnecessarily complex for our needs
    target_path: PathBuf,
    fetch_url: Option<String>,
    push_url: Option<String>,
    shallow_since: Option<NaiveDate>,
    branch_name: String,
    filter: Option<String>,
    // additional repo config keys that will be passed to clone using
    // the -c flag. should be strings of "key=value"
    repo_config: Vec<String>,
    // set of options controlling various flags to pass to clone
    init_opts: HashSet<InitOpt>,
    // additional args to git, before the subcommand
    git_args: Vec<String>,
    // additional args to the clone subcommand
    clone_args: Vec<String>,
}

const DEFAULT_SINGLE_BRANCH: &str = "master";

pub fn parse_shallow_since_date(s: &str) -> Result<NaiveDate> {
    Ok(NaiveDate::parse_from_str(s, "%Y-%m-%d")?)
}

impl Default for CloneBuilder {
    fn default() -> Self {
        Self {
            target_path: PathBuf::new(),
            fetch_url: None,
            push_url: None,
            shallow_since: None,
            branch_name: String::from(DEFAULT_SINGLE_BRANCH),
            filter: None,
            repo_config: Vec::new(),
            init_opts: HashSet::new(),
            git_args: Vec::new(),
            clone_args: Vec::new(),
        }
    }
}

impl CloneBuilder {
    pub fn new(target_path: PathBuf) -> Self {
        Self {
            target_path,
            ..Default::default()
        }
    }

    pub fn build(self, app: Arc<App>) -> Result<(Command, SandboxCommand)> {
        let mut opt_args: Vec<String> = Vec::new();

        if let Some(ss) = self.shallow_since {
            opt_args.push(format!["--shallow-since={}", ss.format("%Y-%m-%d")]);
        }

        opt_args.push("-b".to_string());
        opt_args.push(self.branch_name.to_owned());

        if self.opt_set(InitOpt::Bare) {
            opt_args.push(String::from("--bare"));
        }

        if let Some(f) = self.filter.to_owned() {
            opt_args.push(format!["--filter={}", f]);
        }

        if !self.opt_set(InitOpt::FollowTags) {
            opt_args.push(String::from("--no-tags"));
        }

        if self.opt_set(InitOpt::NoCheckout) {
            opt_args.push(String::from("--no-checkout"));
        }

        if self.opt_set(InitOpt::Sparse) {
            opt_args.push(String::from("--sparse"));
        }

        if self.opt_set(InitOpt::Progress) {
            opt_args.push(String::from("--progress"));
        }

        if let Some(push_url) = self.push_url.to_owned() {
            opt_args.push(String::from("-c"));
            opt_args.push(format!["remote.origin.pushUrl={}", push_url]);
        }

        for kv in self.repo_config {
            opt_args.push(String::from("-c"));
            opt_args.push(kv);
        }

        let fetch_url = self
            .fetch_url
            .ok_or_else(|| anyhow::anyhow!("Fetch URL not provided"))?;

        let (mut cmd, scmd) = git_helper::git_command(app)?;
        cmd.args(self.git_args)
            .arg("clone")
            .args(opt_args)
            .args(self.clone_args)
            .arg(fetch_url)
            .arg(self.target_path);

        Ok((cmd, scmd))
    }

    fn opt_set(&self, opt: InitOpt) -> bool {
        self.init_opts.contains(&opt)
    }

    pub fn shallow_since(&mut self, d: NaiveDate) -> &mut Self {
        self.shallow_since = Some(d);
        self
    }

    pub fn branch(&mut self, name: String) -> &mut Self {
        self.branch_name = name;
        self
    }

    pub fn push_url(&mut self, url: String) -> &mut Self {
        self.push_url = Some(url);
        self
    }

    #[allow(dead_code)]
    pub fn sparse(&mut self, b: bool) -> &mut Self {
        self.add_or_remove_init_opt(InitOpt::Sparse, b);
        self
    }

    #[allow(dead_code)]
    pub fn bare(&mut self, b: bool) -> &mut Self {
        self.add_or_remove_init_opt(InitOpt::Bare, b);
        self
    }

    pub fn filter(&mut self, spec: String) -> &mut Self {
        self.filter = Some(spec);
        self
    }

    #[allow(dead_code)]
    pub fn follow_tags(&mut self, b: bool) -> &mut Self {
        self.add_or_remove_init_opt(InitOpt::FollowTags, b);
        self
    }

    #[allow(dead_code)]
    pub fn no_checkout(&mut self, b: bool) -> &mut Self {
        self.add_or_remove_init_opt(InitOpt::NoCheckout, b);
        self
    }

    pub fn fetch_url(&mut self, ourl: String) -> &mut Self {
        self.fetch_url = Some(ourl);
        self
    }

    // small helper to avoid repetition, if add is true, then add the
    // opt to the set, otherwise remove it
    fn add_or_remove_init_opt(&mut self, opt: InitOpt, add: bool) -> &mut Self {
        if add {
            self.init_opts.insert(opt);
        } else {
            self.init_opts.remove(&opt);
        }
        self
    }

    pub fn init_opts<I>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = InitOpt>,
    {
        for i in args.into_iter() {
            self.init_opts.insert(i);
        }
        self
    }

    #[allow(dead_code)]
    pub fn add_git_args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<String>,
    {
        for arg in args {
            self.git_args.push(arg.as_ref().to_owned());
        }
        self
    }

    #[allow(dead_code)]
    pub fn add_repo_config<K, V>(&mut self, k: K, v: V) -> &mut Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.repo_config.push(format!["{}={}", k.into(), v.into()]);
        self
    }

    pub fn add_clone_arg<I>(&mut self, arg: I) -> &mut Self
    where
        I: Into<String>,
    {
        self.clone_args.push(arg.into());
        self
    }

    pub fn add_clone_args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for arg in args {
            self.add_clone_arg(arg);
        }
        self
    }
}

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
    pub sync_mode: SyncMode,
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
            sync_mode: SyncMode::Incremental,
        }
    }
}

/// Entrypoint for clone operations.
#[tracing::instrument]
pub fn run(
    sparse_repo_path: PathBuf,
    clone_args: CloneArgs,
    template: Option<ClonedRepoTemplate>,
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
        sync_mode,
    } = clone_args;

    let origin = match origin {
        Some(origin) => origin,
        None => bail!("Clone does not have a valid origin"),
    };

    if sparse_repo_path.is_dir() {
        bail!("{} already exists", sparse_repo_path.display());
    }

    //create the sparse repo dir, so other clones don't use the same name
    std::fs::create_dir_all(&sparse_repo_path).context("Failed to create repo directory")?;

    let mut tmp_sparse_repo_path = PathBuf::from(app.sandbox().path());
    tmp_sparse_repo_path.push("tmp_sparse_repo");

    let configure_repo_then_move_in_place = || -> Result<()> {
        let template = match origin {
            Origin::Local(dense_repo_path) => {
                tracing::info!(path = ?dense_repo_path, "Cloning from local path");
                clone_local(
                    &dense_repo_path,
                    &tmp_sparse_repo_path,
                    &branch,
                    copy_branches,
                    days_of_history,
                    app.clone(),
                )?;

                template
            }
            Origin::Remote(url) => {
                tracing::info!(?url, "Cloning from remote");
                clone_remote(
                    url.clone(),
                    &tmp_sparse_repo_path,
                    &branch,
                    days_of_history,
                    app.clone(),
                )?;

                let template = template.or_else(|| ClonedRepoTemplate::from_url(url.clone()));
                if let Some(template) = template {
                    info!(?template, %url, "Using repo template for url");
                }
                template
            }
        };

        set_up_sparse_repo(
            &tmp_sparse_repo_path,
            projects_and_targets,
            template,
            sync_mode,
            app.clone(),
        )?;

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
    let sparse_repo = Repository::open(sparse_repo_path).context("Opening sparse repo")?;

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

    set_up_remotes(&dense_repo, &sparse_repo, branch, app)
        .context("Failed to set up the remotes")?;

    copy_dense_config(&dense_repo, &sparse_repo)
        .context("failed to copy config from dense repo")?;

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
    template: Option<ClonedRepoTemplate>,
    sync_mode: SyncMode,
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
    // before running rest of setup config worktree view to be filtered
    let working_tree = repo.working_tree()?;
    working_tree.set_filter_config(true)?;
    let head_commit = repo.get_head_commit().context("Resolving head commit")?;
    let target_set = compute_and_store_initial_selection(&repo, projects_and_targets, template)?;
    debug!(target_set = ?target_set, "Complete target set");
    repo.set_bazel_oneshot_resolution(sync_mode == SyncMode::OneShot)?;

    let odb = if repo.get_bazel_oneshot_resolution()? {
        None
    } else {
        Some(RocksDBCache::new(repo.underlying()))
    };
    repo.sync(
        head_commit.id(),
        &target_set,
        false,
        app,
        odb.as_ref(),
        None,
    )
    .context("Sync failed")?;

    repo.working_tree()?.write_sync_point_ref()?;

    info!("Writing git config to support instrumentation");
    repo.write_git_config_to_support_instrumentation()
        .context("Could not write git config to support instrumentation")?;

    set_up_bazel_preflight_script(sparse_repo_path)?;

    Ok(())
}

fn compute_and_store_initial_selection(
    repo: &Repo,
    projects_and_targets: Vec<String>,
    template: Option<ClonedRepoTemplate>,
) -> Result<TargetSet> {
    let mut selections = repo.selection_manager()?;
    let operations = projects_and_targets
        .iter()
        .map(|value| Operation::new(OperationAction::default_add(), value))
        .collect::<Vec<Operation>>();

    // FIXME: ideally, we would check to make sure there is no `focus`
    // directory, aand then create `focus/mandatory.projects.json` (and add it to
    // the gitignore), but we don't have any facilities for modifying the
    // mandatory projects programmatically.
    let operations = match template {
        None => operations,
        Some(template) => operations
            .into_iter()
            .chain(
                template
                    .entries()
                    .into_iter()
                    .map(|entry| Operation::new(OperationAction::default_add(), entry)),
            )
            .collect(),
    };

    let result = selections.process(&operations)?;
    if !result.is_success() {
        bail!("Selecting projects and targets failed");
    }
    selections.save()?;
    let selection = selections.computed_selection()?;

    let target_set = selections.compute_complete_target_set()?;
    debug!(target_set = ?target_set, project_and_targets = ?projects_and_targets, selection_projects = ?selection.projects, selection_targets = ?selection.targets, "computing the target set");

    // For open-source projects, it's likely that `.focus` will be created in
    // their working tree and not ignored by default, which means that the next
    // `focus add`/`focus sync` attempt will fail because the working tree is
    // not clean. Improve the experience by ignoring `.focus` by default.
    let info_dir = repo.git_dir().join("info");
    std::fs::create_dir_all(&info_dir).context("Creating .git/info")?;
    let exclude_path = info_dir.join("exclude");
    let mut exclude_file = OpenOptions::new()
        .append(true)
        .open(exclude_path)
        .context("Opening .git/info/exclude")?;
    writeln!(exclude_file, "/.focus/").context("Writing initial .git/info/exclude")?;

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

    // Check if local dense repo has a main branch that's out of date
    if source_url.scheme().eq_ignore_ascii_case("file") {
        let source_path = Path::new(source_url.path());
        let repo = Repo::open(source_path, app.clone()).with_context(|| {
            format!("Could not open the dense repo to check date of {}", branch)
        })?;
        let main_branch = repo
            .underlying()
            .find_branch(branch, git2::BranchType::Local)
            .context("Failed to find main branch")?;
        let main_branch_commit_id = main_branch
            .get()
            .peel_to_commit()
            .context("Failed to peel to commit of main branch")?;
        let main_branch_tip_date = DateTime::from_utc(
            NaiveDateTime::from_timestamp(main_branch_commit_id.time().seconds(), 0),
            Utc,
        )
        .date();

        let shallow_since_date = focus_util::time::date_at_day_in_past(days_of_history)?;
        if days_of_history > 0 && main_branch_tip_date < shallow_since_date {
            bail!("Your main branch {} is older than the specified shallow date: {}. Run a `git pull` in your dense repo to update it!", branch, shallow_since_datestamp)
        }
    }

    let mut builder = CloneBuilder::new(destination_path.into());
    builder
        .fetch_url(source_url.as_str().into())
        .no_checkout(true)
        .follow_tags(false)
        .branch(branch.into());

    if days_of_history > 0 {
        builder.add_clone_arg(format!("--shallow-since={}", shallow_since_datestamp));
    }

    if !copy_branches {
        builder.add_clone_arg("--single-branch");
    }
    run_clone(builder, app)?;
    Ok(())
}

fn set_up_remotes(
    dense_repo: &Repository,
    sparse_repo: &Repository,
    main_branch_name: &str,
    app: Arc<App>,
) -> Result<()> {
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
        let maybe_fetch_url = if let Some(maybe_url) = dense_remote.url() {
            maybe_url
        } else {
            bail!("Dense remote '{}' has no URL", remote_name);
        };

        let maybe_push_url = dense_remote.pushurl().unwrap_or(maybe_fetch_url);
        debug!(?remote_name, fetch_url = ?maybe_fetch_url, push_url = ?maybe_push_url, "Setting up remote");

        let maybe_fetch_url = match Url::parse(maybe_fetch_url) {
            Ok(mut url) => {
                if let Some(host) = url.host() {
                    if cfg!(feature = "twttr") {
                        // Apply Twitter-specific remote treatment.
                        if host.to_string().eq_ignore_ascii_case("git.twitter.biz") {
                            // If the path for the fetch URL does not begin with '/ro', add that prefix.
                            if !url.path().starts_with("/ro") {
                                url.set_path(&format!("/ro{}", url.path()));
                            }
                        }
                    }
                }
                url.as_str().to_owned()
            }
            Err(_) => {
                info!(
                    "Fetch URL ('{}') for remote {} is not a URL",
                    maybe_fetch_url, remote_name
                );
                maybe_fetch_url.to_owned()
            }
        };
        if Url::parse(maybe_push_url).is_err() {
            info!(
                "Push URL ('{}') for remote {} is not a URL",
                maybe_push_url, remote_name
            )
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
            remote_name, maybe_fetch_url, maybe_push_url
        );
        sparse_repo
            .remote_with_fetch(
                remote_name,
                maybe_fetch_url.as_str(),
                &format!(
                    "refs/heads/{main_branch_name}:refs/remotes/{remote_name}/{main_branch_name}"
                ),
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
            .remote_set_pushurl(remote_name, Some(maybe_push_url))
            .with_context(|| {
                format!(
                    "Configuring push URL for remote {} in the sparse repo failed",
                    &remote_name
                )
            })?;
    }
    Ok(())
}

fn copy_dense_config(dense_repo: &Repository, sparse_repo: &Repository) -> Result<()> {
    let dense_cfg = dense_repo
        .config()
        .context("failed to get dense repo config")?
        .open_level(git2::ConfigLevel::Local)
        .context("failed to open level Local in dense repo config")?;

    let mut sparse_cfg = sparse_repo
        .config()
        .context("failed to get sparse repo config")?
        .open_level(git2::ConfigLevel::Local)
        .context("failed to open level Local in sparse repo config")?;

    for k in ["ci.alt.remote", "ci.alt.enabled"] {
        if let Ok(v) = dense_cfg.get_string(k) {
            if v.is_empty() {
                break;
            }
            sparse_cfg.set_str(k, &v).with_context(|| {
                format!(
                    "failed to set key {:#?} value {:#?} from dense repo in sparse repo",
                    k, &v
                )
            })?;
        }
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
        cmd.current_dir(sparse_repo).arg("fetch").arg("origin"),
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

        if days_of_history > 0 {
            if dense_commit_date > shallow_since_datestamp {
                valid_local_branches.push((name.to_owned(), dense_commit.to_owned()));
            } else {
                warn!(
                    "Branch {} is older than the configured limit ({}). Rebase it if you would like it to be included in the new repo.",
                    name, shallow_since_datestamp
                );
            }
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

#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::EnumVariantNames,
    strum_macros::IntoStaticStr,
    strum_macros::EnumIter,
)]
#[strum(serialize_all = "snake_case")]
pub enum ClonedRepoTemplate {
    Bazel,
    Envoy,
    None,
}

impl ClonedRepoTemplate {
    pub fn from_url(url: Url) -> Option<Self> {
        match url.domain() {
            Some("github.com") => {
                let path = url.path();
                let path = path.strip_suffix('/').unwrap_or(path);
                match path {
                    "/bazelbuild/bazel" => Some(Self::Bazel),
                    "/envoyproxy/envoy" => Some(Self::Envoy),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub fn entries(&self) -> impl IntoIterator<Item = &'static str> {
        match self {
            ClonedRepoTemplate::Bazel => {
                vec!["directory:third_party", "directory:tools"]
            }
            ClonedRepoTemplate::Envoy => {
                vec!["directory:bazel", "directory:api", "directory:tools"]
            }
            ClonedRepoTemplate::None => vec![],
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{clone::ClonedRepoTemplate, testing::integration::RepoPairFixture};
    use focus_internals::target::Target;
    use focus_testing::init_logging;

    use anyhow::Result;
    use url::Url;

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

    #[test]
    fn test_template_from_url() -> Result<()> {
        assert_eq!(
            ClonedRepoTemplate::from_url(Url::parse("https://github.com/envoyproxy/envoy")?),
            Some(ClonedRepoTemplate::Envoy)
        );
        assert_eq!(
            ClonedRepoTemplate::from_url(Url::parse("https://github.com/envoyproxy/envoy/")?),
            Some(ClonedRepoTemplate::Envoy)
        );
        assert_eq!(
            ClonedRepoTemplate::from_url(Url::parse("https://gitlab.com/envoyproxy/envoy/")?),
            None
        );
        assert_eq!(
            ClonedRepoTemplate::from_url(Url::parse("https://github.com/kubernetes/kubernetes")?),
            None
        );

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

    use crate::testing::integration::{configure_ci_for_dense_repo, RepoPairFixture};
    use anyhow::{bail, Context, Result};
    use assert_cmd::prelude::OutputAssertExt;
    use focus_internals::model::repo::Repo;
    use focus_testing::init_logging;
    use focus_util::app::App;
    use git2::Repository;
    use tracing::info;

    const MAIN_BRANCH_NAME: &str = "main";

    #[test]
    fn local_clone_smoke_test() -> Result<()> {
        init_logging();
        let fixture = RepoPairFixture::new()?;
        configure_ci_for_dense_repo(&fixture)?;

        let git_exe = fixture.app.git_binary();

        // Set up a remote that mimics source so that we can check that the setting of fetch and push URLs.
        git_exe
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
            "https://git.twitter.biz/focus-test-repo-ci"
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
                        .strip_prefix(outlining_tree_path)
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
                        .strip_prefix(working_tree_path)
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
