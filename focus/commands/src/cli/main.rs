#![allow(clippy::too_many_arguments)]
// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{
    convert::TryFrom,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::Instant,
};

use anyhow::{bail, Context, Result};
use clap::Parser;
use focus_migrations::production::perform_pending_migrations;
use focus_testing::GitBinary;
use git2::Repository;

use focus_util::{
    app::{App, ExitCode},
    git_helper::{self, GitVersion},
    lock_file::LockFile,
    paths, sandbox,
    time::FocusTime,
};

use focus_internals::{target::TargetTypes, tracker::Tracker};
use focus_operations::{
    clone::{CloneArgs, ClonedRepoTemplate},
    maintenance::{self, ScheduleOpts},
    project::lint,
    selection::save,
    sync::SyncMode,
};
use strum::VariantNames;
use termion::{color, style};
use tracing::{debug, debug_span, error, info};

#[derive(Parser, Clone, Debug)]
struct NewArgs {
    /// Path to the repository to clone.
    #[clap(long, default_value = "~/workspace/source")]
    dense_repo: String,

    /// Path where the new sparse repository should be created.
    #[clap(parse(from_os_str))]
    sparse_repo: PathBuf,

    /// The name of the branch to clone.
    #[clap(short, long, default_value = "master")]
    branch: String,

    /// Days of history to maintain in the sparse repo.
    #[clap(long, default_value = "90")]
    days_of_history: u64,

    /// Copy only the specified branch rather than all local branches.
    #[clap(long, parse(try_from_str), default_value = "true")]
    copy_branches: bool,

    /// Initial projects and targets to add to the repo.
    projects_and_targets: Vec<String>,

    #[clap(long, possible_values = ClonedRepoTemplate::VARIANTS)]
    template: Option<ClonedRepoTemplate>,
}

#[derive(Parser, Clone, Debug)]
enum Subcommand {
    /// Create a sparse clone from named layers or ad-hoc build targets
    New(NewArgs),

    /// Deprecated; use `focus new` instead.
    #[clap(hide = true)]
    Clone(NewArgs),

    /// Update the sparse checkout to reflect changes to the build graph.
    Sync {
        /// Path to the sparse repository.
        #[clap(parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,

        /// Sync in one-shot, skipping the cache and invoking the underlying resolver once.
        #[clap(long = "one-shot")]
        one_shot: bool,
    },

    /// Interact with repos configured on this system. Run `focus repo help` for more information.
    Repo {
        #[clap(subcommand)]
        subcommand: RepoSubcommand,
    },

    /// Add projects and targets to the selection.
    Add {
        /// Project and targets to add to the selection.
        projects_and_targets: Vec<String>,

        /// Select projects to add interactively.
        #[clap(short = 'i', long = "interactive")]
        interactive: bool,

        /// In interactive mode, include all targets in the repository in the
        /// search results.
        #[clap(short = 'a', long = "all", requires("interactive"))]
        search_all_targets: bool,

        /// Add the immediate targets and projects of projects to the selection, not the projects themselves.
        #[clap(long = "unroll")]
        unroll: bool,
    },

    /// Remove projects and targets from the selection.
    #[clap(visible_alias("rm"))]
    Remove {
        /// Project and targets to remove from the selection
        projects_and_targets: Vec<String>,

        /// Remove all targets and projects from the selection
        #[clap(short = 'a', long = "all")]
        all: bool,
    },

    /// Display which projects and targets are selected.
    Status {
        ///Unwrap all projects until only targets are displayed
        #[clap(long = "targets")]
        targets: bool,

        //Include only the types of targets specified
        #[clap(short = 't', long = "types", arg_enum)]
        target_types: Vec<TargetTypes>,
    },

    /// List available projects.
    Projects {},

    /// Interact with project definitions
    Project {
        #[clap(subcommand)]
        subcommand: ProjectSubcommand,
    },

    /// Detect whether there are changes to the build graph (used internally)
    DetectBuildGraphChanges {
        /// Path to the repository.
        #[clap(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        /// Whether to treat build graph changes as warnings rather than errors; if true (the default), we should never exit with a non-zero status code in normal operation
        #[clap(long, default_value = "true")]
        advisory: bool,

        /// Arguments passed by the wrapper (a wrapper of `bazel` or otherwise)
        args: Vec<String>,
    },

    /// Utility methods for listing and expiring outdated refs. Used to maintain a time windowed
    /// repository.
    Refs {
        #[clap(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        #[clap(subcommand)]
        subcommand: RefsSubcommand,
    },

    /// Manage branches and branch prefixes in the repo
    Branch {
        /// The repo path to manage branches for
        #[clap(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        #[clap(long, default_value = "origin")]
        remote_name: String,

        #[clap(subcommand)]
        subcommand: BranchSubcommand,
    },

    #[clap(hide = true)]
    Maintenance {
        /// The git config key to look for paths of repos to run maintenance in. Defaults to
        /// 'maintenance.repo'
        #[clap(long, default_value=focus_operations::maintenance::DEFAULT_CONFIG_KEY, global = true)]
        git_config_key: String,

        #[clap(subcommand)]
        subcommand: MaintenanceSubcommand,
    },

    /// git-trace allows one to transform the output of GIT_TRACE2_EVENT data into a format
    /// that the chrome://tracing viewer can understand and display. This is a convenient way
    /// to analyze the timing and call tree of a git command.
    ///
    /// For example, to analyze git gc:
    /// ```
    /// $ GIT_TRACE2_EVENT=/tmp/gc.json git gc
    /// $ focus git-trace /tmp/gc.json /tmp/chrome-trace.json
    /// ````
    /// Then open chrome://tracing in your browser and load the /tmp/chrome-trace.json flie.
    GitTrace { input: PathBuf, output: PathBuf },

    /// Upgrade the repository by running outstanding migration steps.
    Upgrade {
        #[clap(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,
    },

    /// Interact with the on-disk focus index.
    Index {
        #[clap(subcommand)]
        subcommand: IndexSubcommand,
    },

    /// Interact with the project pattern cache.
    ProjectCache {
        #[clap(subcommand)]
        subcommand: ProjectCacheSubcommand,
    },

    /// Called by a git hook to trigger certain actions after a git event such as
    /// merge completion or checkout
    Event { args: Vec<String> },

    /// Print the version of Focus
    Version,

    /// Control automatic background synchronization
    Background {
        #[clap(subcommand)]
        subcommand: BackgroundSubcommand,
    },

    /// Incorporate changes from `prefetch` into the current branch.
    Pull,

    Selection {
        #[clap(subcommand)]
        subcommand: SelectionSubcommand,
    },
}

/// Helper method to extract subcommand name. Tool insights client uses this to set
/// feature name.
fn feature_name_for(subcommand: &Subcommand) -> String {
    match subcommand {
        Subcommand::New { .. } | Subcommand::Clone { .. } => "new".to_string(),
        Subcommand::Sync { .. } => "sync".to_string(),
        Subcommand::Repo { subcommand } => match subcommand {
            RepoSubcommand::List { .. } => "repo-list".to_string(),
            RepoSubcommand::Repair { .. } => "repo-repair".to_string(),
            RepoSubcommand::Register { .. } => "repo-register".to_string(),
        },
        Subcommand::Add { .. } => "add".to_string(),
        Subcommand::Remove { .. } => "remove".to_string(),
        Subcommand::Status { .. } => "status".to_string(),
        Subcommand::Projects { .. } => "projects".to_string(),
        Subcommand::Project { subcommand } => match subcommand {
            ProjectSubcommand::Lint { .. } => "project-lint".to_string(),
        },
        Subcommand::DetectBuildGraphChanges { .. } => "detect-build-graph-changes".to_string(),
        Subcommand::Refs { subcommand, .. } => match subcommand {
            RefsSubcommand::Delete { .. } => "refs-delete".to_string(),
            RefsSubcommand::ListExpired { .. } => "refs-list-expired".to_string(),
            RefsSubcommand::ListCurrent { .. } => "refs-list-current".to_string(),
        },
        Subcommand::Branch { subcommand, .. } => match subcommand {
            BranchSubcommand::List { .. } => "branch-list".to_string(),
            BranchSubcommand::Search { .. } => "branch-search".to_string(),
            BranchSubcommand::Add { .. } => "branch-add".to_string(),
        },
        Subcommand::Maintenance { subcommand, .. } => match subcommand {
            MaintenanceSubcommand::Run { .. } => "maintenance-run".to_string(),
            MaintenanceSubcommand::Register { .. } => "maintenance-register".to_string(),
            MaintenanceSubcommand::SetDefaultConfig { .. } => {
                "maintenance-set-default-config".to_string()
            }
            MaintenanceSubcommand::SandboxCleanup { .. } => {
                "maintenance-sandbox-cleanup".to_string()
            }
            MaintenanceSubcommand::Schedule { subcommand } => match subcommand {
                MaintenanceScheduleSubcommand::Enable { .. } => {
                    "maintenance-schedule-enable".to_string()
                }
                MaintenanceScheduleSubcommand::Disable { .. } => {
                    "maintenance-schedule-disable".to_string()
                }
            },
        },
        Subcommand::GitTrace { .. } => "git-trace".to_string(),
        Subcommand::Upgrade { .. } => "upgrade".to_string(),
        Subcommand::Index { subcommand } => match subcommand {
            IndexSubcommand::Clear { .. } => "index-clear".to_string(),
            IndexSubcommand::CalculateChurn { .. } => "index-calculate-churn".to_string(),
            IndexSubcommand::Fetch { .. } => "index-fetch".to_string(),
            IndexSubcommand::Get { .. } => "index-get".to_string(),
            IndexSubcommand::Generate { .. } => "index-generate".to_string(),
            IndexSubcommand::Hash { .. } => "index-hash".to_string(),
            IndexSubcommand::Push { .. } => "index-push".to_string(),
            IndexSubcommand::Resolve { .. } => "index-resolve".to_string(),
        },
        Subcommand::ProjectCache { subcommand } => match subcommand {
            ProjectCacheSubcommand::Push { .. } => "project-cache-push".to_string(),
        },
        Subcommand::Event { args } => {
            let mut temp_args = args.to_owned();
            temp_args.insert(0, "event".to_string());
            temp_args.join("-")
        }
        Subcommand::Version => "version".to_string(),
        Subcommand::Background { subcommand } => match subcommand {
            BackgroundSubcommand::Enable { .. } => "background-enable".to_string(),
            BackgroundSubcommand::Disable { .. } => "background-disable".to_string(),
            BackgroundSubcommand::Sync { .. } => "background-sync".to_string(),
        },
        Subcommand::Pull => "pull".to_string(),
        Subcommand::Selection { subcommand } => match subcommand {
            SelectionSubcommand::Save { .. } => "selection-save".to_string(),
        },
    }
}

#[derive(Parser, Clone, Debug)]
enum MaintenanceSubcommand {
    /// Runs global (i.e. system-wide) git maintenance tasks on repositories listed in
    /// the $HOME/.gitconfig's `maintenance.repo` multi-value config key. This command
    /// is usually run by a system-specific scheduler (eg. launchd) so it's unlikely that
    /// end users would need to invoke this command directly.
    Run {
        /// The absolute path to the git binary to use. If not given, the default MDE path
        /// will be used.
        #[clap(long, default_value = maintenance::DEFAULT_GIT_BINARY_PATH_FOR_SCHEDULED_JOBS, env = "FOCUS_GIT_BINARY_PATH")]
        git_binary_path: PathBuf,

        /// The git config file to use to read the list of repos to run maintenance in. If not
        /// given, then use the default 'global' config which is usually $HOME/.gitconfig.
        #[clap(long, env = "FOCUS_GIT_CONFIG_PATH")]
        git_config_path: Option<PathBuf>,

        /// run maintenance on repos tracked by focus rather than reading from the
        /// git global config file
        #[clap(long, conflicts_with = "git-config-path", env = "FOCUS_TRACKED")]
        tracked: bool,

        /// The time period of job to run
        #[clap(
            long,
            possible_values=focus_operations::maintenance::TimePeriod::VARIANTS,
            default_value="hourly",
            env = "FOCUS_TIME_PERIOD"
        )]
        time_period: focus_operations::maintenance::TimePeriod,
    },

    SetDefaultConfig {},

    Register {
        #[clap(long, parse(from_os_str))]
        repo_path: Option<PathBuf>,

        #[clap(long, parse(from_os_str))]
        git_config_path: Option<PathBuf>,
    },

    Schedule {
        #[clap(subcommand)]
        subcommand: MaintenanceScheduleSubcommand,
    },

    SandboxCleanup {
        /// Sandboxes older than this many hours will be deleted automatically.
        /// if 0 then time based cleanup is not performed and we just go by
        /// max_num_sandboxes.
        #[clap(long)]
        preserve_hours: Option<u32>,

        /// The maximum number of sandboxes we'll allow to exist on disk.
        /// this is computed after we clean up sandboxes that are older
        /// than preserve_hours
        #[clap(long)]
        max_num_sandboxes: Option<u32>,
    },
}

#[derive(Parser, Clone, Debug)]
enum MaintenanceScheduleSubcommand {
    /// Set up a system-appropriate periodic job (launchctl, systemd, etc.) for running
    /// maintenance tasks on hourly, daily, and weekly bases
    Enable {
        /// The time period of job to schedule
        #[clap(
            long,
            possible_values=focus_operations::maintenance::TimePeriod::VARIANTS,
            default_value="hourly",
            env = "FOCUS_TIME_PERIOD"
        )]
        time_period: focus_operations::maintenance::TimePeriod,

        /// register jobs for all time periods
        #[clap(long, conflicts_with = "time-period", env = "FOCUS_ALL")]
        all: bool,

        /// path to the focus binary, defaults to the current running focus binary
        #[clap(long)]
        focus_path: Option<PathBuf>,

        /// path to git
        #[clap(long, default_value = focus_operations::maintenance::DEFAULT_GIT_BINARY_PATH_FOR_SCHEDULED_JOBS, env = "FOCUS_GIT_BINARY_PATH")]
        git_binary_path: PathBuf,

        /// Normally, we check to see if the scheduled job is already defined and if it is
        /// we do nothing. IF this flag is given, stop the existing job, remove its definition,
        /// rewrite the job manifest (eg. plist) and reload it.
        #[clap(long, env = "FOCUS_FORCE_RELOAD")]
        force_reload: bool,

        /// Add a flag to the maintenance cmdline that will run the tasks against all focus tracked repos
        #[clap(long, env = "FOCUS_TRACKED")]
        tracked: bool,
    },

    /// Unload all the scheduled jobs from the system scheduler (if loaded).
    Disable {
        /// Delete the plist after unloading
        #[clap(long)]
        delete: bool,
    },
}

#[derive(Parser, Clone, Debug)]
enum RepoSubcommand {
    /// List registered repositories
    List {},

    /// Attempt to repair the registry of repositories
    Repair {},

    /// Register (or fix the current registation of) the of the specified repository
    Register {
        #[clap(parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,
    },
}

#[derive(Parser, Clone, Debug)]
enum BranchSubcommand {
    /// List branches in repo
    List {},

    /// Search for branches using a search term
    Search {
        /// Substring used to search refs in the remote server
        ///
        /// Ex:
        ///
        /// 'user' would match with branch 'user' and 'user/branch-1'.
        search_term: String,
    },

    /// Add a branch or set of branches to track from the remote server.
    ///
    /// To track a single branch, run e.g. `focus branch add username/my-feature`.
    /// To track a set of branches, run e.g. `focus branch add 'username/*'`.
    ///
    /// The passed in name should not end in `/`.
    Add { name: String },
}

#[derive(Parser, Clone, Debug)]
enum ProjectSubcommand {
    /// Load projects and then try to parse targets
    Lint {},
}

#[derive(Parser, Clone, Debug)]
enum SelectionSubcommand {
    /// Save your selection to a project
    Save {
        /// Name of project to create or update
        project_name: String,

        /// file root to save new project under: $REPO_ROOT/focus/projects/<project_file>.projects.json
        project_file: Option<String>,

        /// Description of project
        #[clap(long, short = 'd')]
        project_description: Option<String>,
    },
}

#[derive(Parser, Clone, Debug)]
enum RefsSubcommand {
    /// Expires refs that are outside the window of "current refs"
    Delete {
        #[clap(long, default_value = "2021-01-01")]
        cutoff_date: String,

        #[clap(long)]
        use_transaction: bool,

        /// If true, then ensure the merge base falls after the cutoff date.
        /// this avoids the problem of refs that refer to commits that are not
        /// included in master
        #[clap(short = 'm', long = "check-merge-base")]
        check_merge_base: bool,
    },

    ListExpired {
        #[clap(long, default_value = "2021-01-01")]
        cutoff_date: String,

        /// If true, then ensure the merge base falls after the cutoff date.
        /// this avoids the problem of refs that refer to commits that are not
        /// included in master
        #[clap(short = 'm', long = "check-merge-base")]
        check_merge_base: bool,
    },

    /// Output a list of still current (I.e. non-expired) refs
    ListCurrent {
        #[clap(long, default_value = "2021-01-01")]
        cutoff_date: String,

        /// If true, then ensure the merge base falls after the cutoff date.
        /// this avoids the problem of refs that refer to commits that are not
        /// included in master
        #[clap(short = 'm', long = "check-merge-base")]
        check_merge_base: bool,
    },
}

#[derive(Parser, Clone, Debug)]
enum IndexSubcommand {
    /// Clear the on-disk cache.
    Clear {
        /// Path to the sparse repository.
        #[clap(parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,
    },

    /// Calculate statistics on cache invalidation for the projects in the
    /// repository.
    CalculateChurn {
        /// Path to the sparse repository.
        #[clap(long, parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,

        /// The number of commits backwards to examine.
        #[clap(long, default_value = "1000")]
        num_commits: usize,
    },

    /// Fetch the pre-computed index for the repository.
    Fetch {
        /// Path to the sparse repository.
        #[clap(parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,

        /// Force fetching an index, even if index fetching is disabled for this
        /// repository.
        #[clap(short = 'f', long = "force")]
        force: bool,

        /// Override the remote provided in the config.
        #[clap(long)]
        remote: Option<String>,
    },

    Get {
        target: String,
    },

    /// Populate the index with entries for all projects.
    Generate {
        /// Path to the sparse repository.
        #[clap(parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,

        /// If index keys are found to be missing, pause for debugging.
        #[clap(long)]
        break_on_missing_keys: bool,
    },

    /// Calculate and print the content hashes of the provided targets.
    Hash {
        /// The commit at which to hash the provided targets.
        #[clap(long, default_value = "HEAD")]
        commit: String,

        /// The targets to hash.
        targets: Vec<String>,
    },

    /// Generate and push the pre-computed index to the remote store for others
    /// to fetch.
    Push {
        /// Path to the sparse repository.
        #[clap(parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,

        /// The Git remote to push to.
        #[clap(long, default_value = focus_operations::index::INDEX_DEFAULT_REMOTE)]
        remote: String,

        /// Do not actually push the index data to the remote. (It will still be
        /// generated and cached locally.)
        #[clap(short = 'N', long = "dry-run")]
        dry_run: bool,

        /// If index keys are found to be missing, pause for debugging.
        #[clap(long)]
        break_on_missing_keys: bool,
    },

    /// Resolve the targets to their resulting pattern sets.
    Resolve {
        targets: Vec<String>,

        /// If index keys are found to be missing, pause for debugging.
        #[clap(long)]
        break_on_missing_keys: bool,
    },
}

#[derive(Parser, Clone, Debug)]
enum ProjectCacheSubcommand {
    /// Generate project cache data for the given commit and push it to the configured remote.
    Push {
        /// Path to the sparse repository.
        #[clap(long, parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,

        /// The commit to generate cache content for.
        #[clap(long, default_value = "HEAD")]
        commit: String,

        /// Which shard to calculate; note: zero-based.
        #[clap(long)]
        shard_index: usize,

        /// How many shards there are in total.
        #[clap(long)]
        shard_count: usize,
    },
}

#[derive(Parser, Clone, Debug)]
#[allow(clippy::enum_variant_names)]
enum EventSubcommand {
    PostCheckout,
    PostCommit,
    PostMerge,
}

#[derive(Parser, Clone, Debug)]
enum BackgroundSubcommand {
    /// Enable preemptive background synchronization
    Enable {
        /// Path to the sparse repository.
        #[clap(parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,

        /// Idle threshold: how long must the machine be inactive before performing pre-emptive sync? (In milliseconds)
        #[clap(default_value = "30000")]
        idle_period_ms: u64,
    },

    /// Disable preemptive background synchronization.
    Disable {
        /// Path to the sparse repository.
        #[clap(parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,
    },

    /// Manually run a preemptive sync
    Sync {
        /// Path to the sparse repository.
        #[clap(parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,
    },
}

#[derive(Parser, Clone, Debug)]
#[clap(about = "Focused Development Tools")]
struct FocusOpts {
    /// Number of threads to use when performing parallel resolution (where possible).
    #[clap(
        long,
        default_value = "0",
        global = true,
        env = "FOCUS_RESOLUTION_THREADS"
    )]
    resolution_threads: usize,

    /// Change to the provided directory before doing anything else.
    #[clap(
        short = 'C',
        long = "work-dir",
        global = true,
        env = "FOCUS_WORKING_DIRECTORY"
    )]
    working_directory: Option<PathBuf>,

    /// Disables use of ANSI color escape sequences
    #[clap(long, global = true, env = "NO_COLOR")]
    no_color: bool,

    #[clap(subcommand)]
    cmd: Subcommand,
}

fn ensure_directories_exist(tracker: &Tracker) -> Result<()> {
    tracker
        .ensure_directories_exist()
        .context("creating directories for the tracker")?;

    Ok(())
}

fn hold_lock_file(repo: &Path) -> Result<LockFile> {
    let path = repo.join(".focus").join("focus.lock");
    LockFile::new(&path)
}

fn check_compatible_git_version(git_binary: &GitBinary) -> Result<bool> {
    let passed = match GitVersion::current(git_binary)? {
        GitVersion { major, minor, .. } if major >= 2 && minor >= 35 => true,
        GitVersion { major, minor, .. } => {
            error!("Focus requires Git version 2.35 or newer. This system has version {}.{} installed. Please update Git and try again.", major, minor);
            false
        }
    };

    Ok(passed)
}

fn preflight_check(app: Arc<App>) -> Result<bool> {
    check_compatible_git_version(app.git_binary())
}

fn run_subcommand(app: Arc<App>, tracker: &Tracker, options: FocusOpts) -> Result<ExitCode> {
    let cloned_app = app.clone();
    let ti_client = cloned_app.tool_insights_client();
    let feature_name = feature_name_for(&options.cmd);
    ti_client.get_context().set_tool_feature_name(&feature_name);
    let span = debug_span!("Running subcommand", ?feature_name);
    let _guard = span.enter();

    // This is to accomadate the special case for some maintenance run scheduled with launchd,
    // We will want to validate the passed in git binary, not the one in app.
    let run_preflight_check = match options.cmd {
        Subcommand::Maintenance { ref subcommand, .. } => {
            !matches!(subcommand, MaintenanceSubcommand::Run { .. })
        }
        _ => true,
    };

    if run_preflight_check && !preflight_check(app.clone())? {
        return Ok(ExitCode(1));
    };

    if let Subcommand::Clone(_) = &options.cmd {
        eprintln!(
            "{}{}The command `focus clone` is deprecated; use `focus new` instead!{}",
            style::Bold,
            color::Fg(color::Yellow),
            style::Reset,
        );
    }

    match options.cmd {
        Subcommand::New(NewArgs {
            dense_repo,
            sparse_repo,
            branch,
            days_of_history,
            copy_branches,
            projects_and_targets,
            template,
        })
        | Subcommand::Clone(NewArgs {
            dense_repo,
            sparse_repo,
            branch,
            days_of_history,
            copy_branches,
            projects_and_targets,
            template,
        }) => {
            let origin = focus_operations::clone::Origin::try_from(dense_repo.as_str())?;
            let sparse_repo = {
                let current_dir =
                    std::env::current_dir().context("Failed to obtain current directory")?;
                let expanded = paths::expand_tilde(sparse_repo)
                    .context("Failed to expand sparse repo path")?;
                current_dir.join(expanded)
            };

            info!("Cloning {:?} into {}", dense_repo, sparse_repo.display());

            // Add targets length to TI custom map.
            ti_client.get_context().add_to_custom_map(
                "projects_and_targets_count",
                projects_and_targets.len().to_string(),
            );

            let clone_args = CloneArgs {
                origin: Some(origin),
                branch,
                days_of_history,
                copy_branches,
                projects_and_targets,
                ..Default::default()
            };

            focus_operations::clone::run(sparse_repo.clone(), clone_args, template, tracker, app)?;

            perform_pending_migrations(&sparse_repo)
                .context("Performing initial migrations after clone")?;

            Ok(ExitCode(0))
        }
        Subcommand::Sync {
            sparse_repo,
            one_shot,
        } => {
            // TODO: Add total number of paths in repo to TI.
            let sparse_repo =
                paths::find_repo_root_from(app.clone(), paths::expand_tilde(sparse_repo)?)?;
            ensure_repo_compatibility(&sparse_repo)?;

            let _lock_file = hold_lock_file(&sparse_repo)?;
            let mode = if one_shot {
                SyncMode::OneShot
            } else {
                SyncMode::Incremental
            };
            focus_operations::sync::run(&sparse_repo, mode, app)?;
            Ok(ExitCode(0))
        }

        Subcommand::Refs {
            repo: repo_path,
            subcommand,
        } => {
            let sparse_repo = paths::find_repo_root_from(app.clone(), repo_path)?;
            let repo = Repository::open(sparse_repo).context("opening the repo")?;
            match subcommand {
                RefsSubcommand::Delete {
                    cutoff_date,
                    use_transaction,
                    check_merge_base,
                } => {
                    let cutoff = FocusTime::parse_date(cutoff_date)?;
                    focus_operations::refs::expire_old_refs(
                        &repo,
                        cutoff,
                        check_merge_base,
                        use_transaction,
                        app,
                    )?;
                    Ok(ExitCode(0))
                }

                RefsSubcommand::ListExpired {
                    cutoff_date,
                    check_merge_base,
                } => {
                    let cutoff = FocusTime::parse_date(cutoff_date)?;
                    let focus_operations::refs::PartitionedRefNames {
                        current: _,
                        expired,
                    } = focus_operations::refs::PartitionedRefNames::for_repo(
                        &repo,
                        cutoff,
                        check_merge_base,
                    )?;

                    println!("{}", expired.join("\n"));

                    Ok(ExitCode(0))
                }

                RefsSubcommand::ListCurrent {
                    cutoff_date,
                    check_merge_base,
                } => {
                    let cutoff = FocusTime::parse_date(cutoff_date)?;
                    let focus_operations::refs::PartitionedRefNames {
                        current,
                        expired: _,
                    } = focus_operations::refs::PartitionedRefNames::for_repo(
                        &repo,
                        cutoff,
                        check_merge_base,
                    )?;

                    println!("{}", current.join("\n"));

                    Ok(ExitCode(0))
                }
            }
        }

        Subcommand::Branch {
            subcommand,
            repo,
            remote_name,
        } => {
            let repo = paths::find_repo_root_from(app.clone(), repo)?;
            match subcommand {
                BranchSubcommand::List {} => {
                    focus_operations::branch::list(app, repo, &remote_name)?;
                    Ok(ExitCode(0))
                }
                BranchSubcommand::Search { search_term } => {
                    focus_operations::branch::search(app, repo, &remote_name, &search_term)?;
                    Ok(ExitCode(0))
                }
                BranchSubcommand::Add { name } => {
                    focus_operations::branch::add(app, repo, &remote_name, &name)
                }
            }
        }

        Subcommand::Repo { subcommand } => match subcommand {
            RepoSubcommand::List {} => {
                focus_operations::repo::list(tracker)?;
                Ok(ExitCode(0))
            }
            RepoSubcommand::Repair {} => {
                focus_operations::repo::repair(tracker, app)?;
                Ok(ExitCode(0))
            }

            RepoSubcommand::Register { sparse_repo } => {
                focus_operations::repo::register(sparse_repo, tracker, app)?;
                Ok(ExitCode(0))
            }
        },

        Subcommand::DetectBuildGraphChanges {
            repo,
            advisory,
            args,
        } => {
            let repo = paths::find_repo_root_from(app.clone(), paths::expand_tilde(repo)?)?;
            let repo = git_helper::find_top_level(app.clone(), &repo)
                .context("Failed to canonicalize repo path")?;
            focus_operations::detect_build_graph_changes::run(&repo, advisory, args, app)
        }

        Subcommand::Add {
            projects_and_targets,
            interactive,
            search_all_targets,
            unroll,
        } => {
            let sparse_repo = paths::find_repo_root_from(app.clone(), std::env::current_dir()?)?;
            paths::assert_focused_repo(&sparse_repo)?;
            let _lock_file = hold_lock_file(&sparse_repo)?;
            focus_operations::ensure_clean::run(&sparse_repo, app.clone())
                .context("Ensuring working trees are clean failed")?;

            if interactive {
                focus_operations::selection::add_interactive(
                    &sparse_repo,
                    app,
                    search_all_targets,
                    unroll,
                )?;
            } else {
                focus_operations::selection::add(
                    &sparse_repo,
                    true,
                    projects_and_targets,
                    unroll,
                    app,
                )?;
            }
            Ok(ExitCode(0))
        }

        Subcommand::Remove {
            projects_and_targets,
            all,
        } => {
            
            let sparse_repo = paths::find_repo_root_from(app.clone(), std::env::current_dir()?)?;
            let _lock_file = hold_lock_file(&sparse_repo)?;
            focus_operations::ensure_clean::run(&sparse_repo, app.clone())
                .context("Ensuring working trees are clean failed")?;
            focus_operations::selection::remove(&sparse_repo, true, projects_and_targets, all, app)?;            
            Ok(ExitCode(0))
        }

        Subcommand::Status {
            targets,
            target_types,
        } => {
            let sparse_repo = paths::find_repo_root_from(app.clone(), std::env::current_dir()?)?;
            focus_operations::status::run(&sparse_repo, app, targets, target_types)
        }

        Subcommand::Projects {} => {
            let repo = git_helper::find_top_level(app.clone(), std::env::current_dir()?)
                .context("Finding the top level of the repo")?;
            focus_operations::selection::list_projects(&repo, app)?;
            Ok(ExitCode(0))
        }

        Subcommand::Maintenance {
            subcommand,
            git_config_key,
        } => match subcommand {
            MaintenanceSubcommand::Run {
                git_binary_path,
                tracked,
                git_config_path,
                time_period,
            } => {
                let git_binary = GitBinary::from_binary_path(git_binary_path)?;
                if !check_compatible_git_version(&git_binary)? {
                    return Ok(ExitCode(1));
                }

                focus_operations::maintenance::run(
                    focus_operations::maintenance::RunOptions {
                        git_binary: Some(git_binary),
                        git_config_key,
                        git_config_path,
                        tracked,
                    },
                    time_period,
                    tracker,
                    app,
                )?;

                sandbox::cleanup::run_with_default()?;

                Ok(ExitCode(0))
            }

            MaintenanceSubcommand::Register {
                repo_path,
                git_config_path,
            } => {
                let repo_path = match repo_path {
                    Some(path) => Some(paths::find_repo_root_from(app, path)?),
                    None => None,
                };

                focus_operations::maintenance::register(
                    focus_operations::maintenance::RegisterOpts {
                        repo_path,
                        git_config_key,
                        global_config_path: git_config_path,
                    },
                )?;
                Ok(ExitCode(0))
            }

            MaintenanceSubcommand::SetDefaultConfig { .. } => {
                focus_operations::maintenance::set_default_git_maintenance_config(
                    &paths::find_repo_root_from(app, std::env::current_dir()?)?,
                )?;
                Ok(ExitCode(0))
            }

            MaintenanceSubcommand::Schedule { subcommand } => match subcommand {
                MaintenanceScheduleSubcommand::Enable {
                    time_period,
                    all,
                    focus_path,
                    git_binary_path,
                    force_reload,
                    tracked,
                } => {
                    maintenance::schedule_enable(maintenance::ScheduleOpts {
                        time_period: if all { None } else { Some(time_period) },
                        git_path: git_binary_path,
                        focus_path: match focus_path {
                            Some(fp) => fp,
                            None => std::env::current_exe()
                                .context("could not determine current executable path")?,
                        },
                        skip_if_already_scheduled: !force_reload,
                        tracked,
                    })?;
                    Ok(ExitCode(0))
                }

                MaintenanceScheduleSubcommand::Disable { delete } => {
                    maintenance::schedule_disable(delete)?;
                    Ok(ExitCode(0))
                }
            },

            MaintenanceSubcommand::SandboxCleanup {
                preserve_hours,
                max_num_sandboxes,
            } => {
                let config = sandbox::cleanup::Config {
                    preserve_hours: preserve_hours
                        .unwrap_or(sandbox::cleanup::Config::DEFAULT_HOURS),
                    max_num_sandboxes: max_num_sandboxes
                        .unwrap_or(sandbox::cleanup::Config::DEFAULT_MAX_NUM_SANDBOXES),
                    ..sandbox::cleanup::Config::try_from_git_default()?
                };

                sandbox::cleanup::run(&config)?;

                Ok(ExitCode(0))
            }
        },

        Subcommand::GitTrace { input, output } => {
            focus_tracing::Trace::git_trace_from(input)?.write_trace_json_to(output)?;
            Ok(ExitCode(0))
        }

        Subcommand::Upgrade { repo } => {
            focus_migrations::production::perform_pending_migrations(
                paths::find_repo_root_from(app, repo)?.as_path(),
            )
            .context("Failed to upgrade repo")?;

            Ok(ExitCode(0))
        }

        Subcommand::Index { subcommand } => match subcommand {
            IndexSubcommand::Clear { sparse_repo } => {
                let sparse_repo = paths::find_repo_root_from(app, sparse_repo)?;
                focus_operations::index::clear(sparse_repo)?;
                Ok(ExitCode(0))
            }

            IndexSubcommand::CalculateChurn {
                sparse_repo,
                num_commits,
            } => {
                let sparse_repo_path = paths::find_repo_root_from(app.clone(), sparse_repo)?;
                focus_operations::index::print_churn_stats(app, sparse_repo_path, num_commits)?;
                Ok(ExitCode(0))
            }

            IndexSubcommand::Fetch {
                sparse_repo,
                force,
                remote,
            } => {
                let sparse_repo = paths::find_repo_root_from(app.clone(), sparse_repo)?;
                let exit_code = focus_operations::index::fetch(app, sparse_repo, force, remote)?;
                Ok(exit_code)
            }

            IndexSubcommand::Generate {
                sparse_repo,
                break_on_missing_keys,
            } => {
                let sparse_repo = paths::find_repo_root_from(app.clone(), sparse_repo)?;
                let exit_code =
                    focus_operations::index::generate(app, sparse_repo, break_on_missing_keys)?;
                Ok(exit_code)
            }

            IndexSubcommand::Get { target } => {
                let sparse_repo = paths::find_repo_root_from(app.clone(), PathBuf::from("."))?;
                let exit_code = focus_operations::index::get(app, &sparse_repo, &target)?;
                Ok(exit_code)
            }

            IndexSubcommand::Hash { commit, targets } => {
                let sparse_repo = paths::find_repo_root_from(app.clone(), PathBuf::from("."))?;
                let exit_code = focus_operations::index::hash(app, &sparse_repo, commit, &targets)?;
                Ok(exit_code)
            }

            IndexSubcommand::Push {
                sparse_repo,
                remote,
                dry_run,
                break_on_missing_keys,
            } => {
                let sparse_repo = paths::find_repo_root_from(app.clone(), sparse_repo)?;
                let exit_code = focus_operations::index::push(
                    app,
                    sparse_repo,
                    remote,
                    dry_run,
                    break_on_missing_keys,
                )?;
                Ok(exit_code)
            }

            IndexSubcommand::Resolve {
                targets,
                break_on_missing_keys,
            } => {
                let sparse_repo = paths::find_repo_root_from(app.clone(), PathBuf::from("."))?;
                let exit_code = focus_operations::index::resolve(
                    app,
                    &sparse_repo,
                    targets,
                    break_on_missing_keys,
                )?;
                Ok(exit_code)
            }
        },

        Subcommand::ProjectCache { subcommand } => match subcommand {
            ProjectCacheSubcommand::Push {
                sparse_repo,
                commit,
                shard_index,
                shard_count,
            } => {
                let sparse_repo = paths::find_repo_root_from(app.clone(), sparse_repo)?;
                let exit_code = focus_operations::project_cache::push(
                    app,
                    sparse_repo,
                    commit,
                    shard_index,
                    shard_count,
                )?;
                Ok(exit_code)
            }
        },

        Subcommand::Project { subcommand } => match subcommand {
            ProjectSubcommand::Lint {} => {
                let repo = git_helper::find_top_level(app.clone(), std::env::current_dir()?)
                    .context("Finding the top level of the repo")?;
                lint(&repo, app)
            }
        },

        Subcommand::Event { args: _ } => Ok(ExitCode(0)),

        Subcommand::Version => {
            println!("package-name: {}", env!("CARGO_PKG_NAME"));
            println!("build-version: {}", env!("VERGEN_BUILD_SEMVER"));
            println!("commit-timestamp: {}", env!("VERGEN_GIT_COMMIT_TIMESTAMP"));
            println!("commit-sha: {}", env!("VERGEN_GIT_SHA"));
            println!("cargo-features: {}", env!("VERGEN_CARGO_FEATURES"));
            println!(
                "twttr-enabled: {}",
                env!("VERGEN_CARGO_FEATURES")
                    .split(',')
                    .any(|feature| feature == "twttr")
            );
            Ok(ExitCode(0))
        }

        Subcommand::Background { subcommand } => match subcommand {
            BackgroundSubcommand::Enable {
                sparse_repo,
                idle_period_ms,
            } => {
                let sparse_repo = paths::find_repo_root_from(app.clone(), sparse_repo)?;
                focus_operations::background::enable(app, sparse_repo, idle_period_ms)
            }
            BackgroundSubcommand::Disable { sparse_repo } => {
                let sparse_repo = paths::find_repo_root_from(app.clone(), sparse_repo)?;
                focus_operations::background::disable(app, sparse_repo)
            }
            BackgroundSubcommand::Sync { sparse_repo } => {
                let sparse_repo = paths::find_repo_root_from(app.clone(), sparse_repo)?;
                focus_operations::background::sync(app, sparse_repo)
            }
        },
        Subcommand::Pull => {
            let sparse_repo = paths::find_repo_root_from(app.clone(), std::env::current_dir()?)?;
            focus_operations::pull::run(app, sparse_repo)
        }
        Subcommand::Selection { subcommand } => match subcommand {
            SelectionSubcommand::Save {
                project_name,
                project_file,
                project_description,
            } => {
                let sparse_repo =
                    paths::find_repo_root_from(app.clone(), std::env::current_dir()?)?;
                save(
                    &sparse_repo,
                    project_name,
                    project_file,
                    project_description,
                    app,
                )?;
                Ok(ExitCode(0))
            }
        },
    }
}

fn ensure_repo_compatibility(sparse_repo: &Path) -> Result<()> {
    if focus_migrations::production::is_upgrade_required(sparse_repo)
        .context("Failed to determine whether an upgrade is required")?
    {
        bail!(
            "Repo '{}' needs to be upgraded. Please run `focus upgrade`",
            sparse_repo.display()
        );
    }

    Ok(())
}

fn setup_thread_pool(resolution_threads: usize) -> Result<()> {
    if resolution_threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(resolution_threads)
            .build_global()
            .context("Failed to create the task thread pool")?;
    }

    Ok(())
}

// TODO: there needs to be a way to know if we should re-load the plists, (eg. on a version change)
fn setup_maintenance_scheduler(opts: &FocusOpts) -> Result<()> {
    if std::env::var("FOCUS_NO_SCHEDULE").is_ok() {
        return Ok(());
    }

    match opts.cmd {
        Subcommand::New { .. }
        | Subcommand::Clone { .. }
        | Subcommand::Sync { .. }
        | Subcommand::Add { .. }
        | Subcommand::Remove { .. } => {
            focus_operations::maintenance::schedule_enable(ScheduleOpts::default())
        }
        _ => Ok(()),
    }
}

/// Run the main and any destructors. Local variables are not guaranteed to be
/// dropped if `std::process::exit` is called, so make sure to bubble up the
/// return code to the top level, which is the only place in the code that's
/// allowed to call `std::process::exit`.
fn main_and_drop_locals() -> Result<ExitCode> {
    let started_at = Instant::now();
    let options = FocusOpts::parse();

    let FocusOpts {
        resolution_threads,
        working_directory,
        no_color,
        cmd: _,
    } = &options;

    if let Some(working_directory) = working_directory {
        std::env::set_current_dir(working_directory).context("Switching working directory")?;
    }

    let preserve_sandbox = true;

    let app = Arc::from(App::new(
        preserve_sandbox,
        Some(&feature_name_for(&options.cmd)),
        Some(env!("CARGO_PKG_NAME").to_owned()),
        Some(env!("CARGO_PKG_VERSION").to_owned()),
    )?);
    let ti_context = app.tool_insights_client();

    setup_thread_pool(*resolution_threads)?;

    let is_tty = termion::is_tty(&std::io::stdout());

    let sandbox_dir = app.sandbox().path().to_owned();
    let tracker = Tracker::from_config_dir()?;

    let _guard = focus_tracing::init_tracing(focus_tracing::TracingOpts {
        is_tty,
        no_color: *no_color,
        log_dir: Some(sandbox_dir.to_owned()),
    })?;

    info!(path = ?sandbox_dir, "Created sandbox");

    ensure_directories_exist(&tracker).context("Failed to create necessary directories")?;
    let setup_maintenance_task = thread::spawn({
        let options = options.clone();
        move || -> anyhow::Result<()> {
            setup_maintenance_scheduler(&options).context("Failed to setup maintenance scheduler")
        }
    });

    let exit_code = match run_subcommand(app.clone(), &tracker, options) {
        Ok(exit_code) => {
            ti_context
                .get_inner()
                .write_invocation_message(Some(0), None);
            exit_code
        }
        Err(e) => {
            ti_context
                .get_inner()
                .write_invocation_message(Some(1), None);
            return Err(e);
        }
    };

    sandbox::cleanup::run_with_default()?;
    setup_maintenance_task.join().unwrap()?;

    let total_runtime = started_at.elapsed();
    debug!(
        total_runtime_secs = total_runtime.as_secs_f32(),
        "Finished normally"
    );

    Ok(exit_code)
}

fn main() -> Result<()> {
    let ExitCode(exit_code) = main_and_drop_locals()?;
    std::process::exit(exit_code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_cmd_feature_name_for_is_in_right_format() -> Result<()> {
        let event_cmd = Subcommand::Event {
            args: vec![
                "this".to_string(),
                "is".to_string(),
                "an".to_string(),
                "event".to_string(),
                "subcommand".to_string(),
                "teehee".to_string(),
            ],
        };
        let feature_name = feature_name_for(&event_cmd);
        assert_eq!(feature_name, "event-this-is-an-event-subcommand-teehee");
        Ok(())
    }
}
