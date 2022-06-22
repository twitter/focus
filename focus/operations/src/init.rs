// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashSet, path::PathBuf, process::Command, sync::Arc};

use anyhow::{bail, Context, Result};
use chrono::{Datelike, Duration, Local, NaiveDate};
use focus_util::{
    app::App,
    git_helper,
    sandbox_command::{SandboxCommand, SandboxCommandOutput},
};
use tempfile::TempDir;

pub static SOURCE_RO_URL: &str = "https://rogit.twitter.biz/source";
static DEFAULT_SINGLE_BRANCH: &str = "master";

pub fn run_clone(mut clone_builder: CloneBuilder, app: Arc<App>) -> Result<()> {
    (&mut clone_builder).add_clone_args(vec!["--progress"]);
    let (mut cmd, scmd) = clone_builder.build(app)?;

    scmd.ensure_success_or_log(
        &mut cmd,
        SandboxCommandOutput::Stderr,
        "perform clone of the repo",
    )
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

#[allow(clippy::too_many_arguments)]
pub fn run(
    shallow_since: Option<NaiveDate>,
    branch_name: Option<String>,
    filter: Option<String>,
    fetch_url: String,
    push_url: Option<String>,
    target_path: PathBuf,
    init_opts: Vec<InitOpt>,
    app: Arc<App>,
) -> Result<()> {
    if target_path.exists() {
        bail!("Target path {:?} already exists!", target_path)
    }

    let temp_dir = TempDir::new_in(
        target_path
            .parent()
            .expect("could not find parent of target_path"),
    )
    .context("could not create tempdir in parent of given target_path")?;

    let temp_repo_path_buf = temp_dir.path().join("repo");

    let temp_path = temp_repo_path_buf.as_path();

    {
        let mut builder = CloneBuilder::new(temp_repo_path_buf.to_owned());

        let b = &mut builder;

        if let Some(d) = shallow_since {
            b.shallow_since(d);
        }

        if let Some(name) = branch_name {
            b.branch(name);
        }

        if let Some(s) = filter {
            b.filter(s);
        }

        if let Some(push_url) = push_url {
            b.push_url(push_url);
        }

        b.fetch_url(fetch_url).init_opts(init_opts);

        run_clone(builder, app.clone()).context("initial clone of the repo")?;
    }

    {
        let repo = git2::Repository::open(temp_path)
            .context("could not open cloned repo for configuration")?;

        let mut config = repo.config().context("could not get repo config")?;

        config.set_bool("manageconfig.enable", true)?;
        config.set_str("remote.origin.tagOpt", "--no-tags")?;
        config.set_bool("remote.origin.prune", true)?;
        config.set_bool("twitter.federated", true)?;

        repo.remote_add_fetch(
            "origin",
            "+refs/heads/repo.d/master:refs/remotes/origin/repo.d/master",
        )?;
    }

    {
        let (mut cmd, scmd) = git_helper::git_command("Clone repo", app)?;
        scmd.ensure_success_or_log(
            cmd.arg("fetch")
                .arg("--no-tags")
                .arg("origin")
                .current_dir(temp_path),
            SandboxCommandOutput::Stderr,
            "do first fetch in new repo",
        )?;
    }

    std::fs::rename(temp_path, target_path).context("failed to move repo to target location")
}

#[derive(Debug)]
pub struct CloneBuilder {
    // note, to be totally typesafe we'd have a type that represented
    // when the target_path was set, and another to represent another
    // where it was unset, but that's unnecessarily complex for our needs
    target_path: PathBuf,
    fetch_url: String,
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

pub fn parse_shallow_since_date(s: &str) -> Result<NaiveDate> {
    Ok(NaiveDate::parse_from_str(s, "%Y-%m-%d")?)
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

        let (mut cmd, scmd) = git_helper::git_command("Clone repo", app)?;

        cmd.args(self.git_args)
            .arg("clone")
            .args(opt_args)
            .args(self.clone_args)
            .arg(self.fetch_url)
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
        self.fetch_url = ourl;
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

static MASTER_HISTORY_WINDOW_DAYS: i64 = 90;

fn default_master_history_window() -> Duration {
    Duration::days(MASTER_HISTORY_WINDOW_DAYS)
}

fn default_shallow_since() -> NaiveDate {
    let t = Local::now();
    let nd = NaiveDate::from_ymd(t.year(), t.month(), t.day());

    nd - default_master_history_window()
}

impl Default for CloneBuilder {
    fn default() -> Self {
        Self {
            target_path: PathBuf::new(),
            fetch_url: String::from(SOURCE_RO_URL),
            push_url: None,
            shallow_since: Some(default_shallow_since()),
            branch_name: String::from(DEFAULT_SINGLE_BRANCH),
            filter: None,
            repo_config: Vec::new(),
            init_opts: HashSet::new(),
            git_args: Vec::new(),
            clone_args: Vec::new(),
        }
    }
}
