#![allow(clippy::too_many_arguments)]

use std::{
    convert::TryFrom,
    env,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use clap::Parser;
use env_logger::{self, Env};
use git2::Repository;

use focus_internals::{
    app::App,
    coordinate::Coordinate,
    model::LayerSets,
    tracker::Tracker,
    util::{backed_up_file::BackedUpFile, git_helper, paths, time::FocusTime},
};
use subcommands::{init::InitOpt, maintenance::launchd};

use crate::subcommands::{
    adhoc, init, layer, maintenance,
    maintenance::{TimePeriod},
    refs,
};
use strum::{VariantNames, IntoEnumIterator};

mod subcommands;

#[derive(Parser, Debug)]
enum Subcommand {
    /// Create a sparse clone from named layers or ad-hoc build coordinates
    Clone {
        /// Copy only the specified branch rather than all local branches.
        #[clap(long)]
        single_branch: bool,

        /// Path to the dense repository to base the clone on.
        #[clap(long, parse(from_os_str), default_value = "~/workspace/source")]
        dense_repo: PathBuf,

        /// Path where the new sparse repository should be created.
        #[clap(parse(from_os_str))]
        sparse_repo: PathBuf,

        /// The name of the branch to clone.
        #[clap(long, default_value = "master")]
        branch: String,

        /// Named layers and ad-hoc coordinates to include in the clone. Named layers are loaded from the dense repo's `focus/projects` directory.
        coordinates_and_layers: Vec<String>,
    },

    /// Update the sparse checkout to reflect changes to the build graph.
    Sync {
        /// Path to the sparse repository.
        #[clap(parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,
    },

    /// Interact with repos configured on this system. Run `focus repo help` for more information.
    Repo {
        #[clap(subcommand)]
        subcommand: RepoSubcommand,
    },

    /// Interact with the stack of selected layers. Run `focus layer help` for more information.
    Layer {
        /// Path to the repository.
        #[clap(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        #[clap(subcommand)]
        subcommand: LayerSubcommand,
    },

    /// Interact with the ad-hoc coordinate stack. Run `focus adhoc help` for more information.
    Adhoc {
        /// Path to the repository.
        #[clap(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        #[clap(subcommand)]
        subcommand: AdhocSubcommand,
    },

    /// Detect whether there are changes to the build graph (used internally)
    DetectBuildGraphChanges {
        // Path to the repository.
        #[clap(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,
    },

    /// Utility methods for listing and expiring outdated refs. Used to maintain a time windowed
    /// repository.
    Refs {
        #[clap(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        #[clap(subcommand)]
        subcommand: RefsSubcommand,
    },

    /// Set up an initial clone of the repo from the remote
    Init {
        /// By default we take 90 days of history, pass a date with this option
        /// if you want a different amount of history
        #[clap(long, parse(try_from_str = init::parse_shallow_since_date))]
        shallow_since: Option<NaiveDate>,

        /// This command will only ever clone a single ref, by default this is
        /// "master". If you wish to clone a different branch, then use this option
        #[clap(long, default_value = "master")]
        branch_name: String,

        #[clap(long)]
        no_checkout: bool,

        /// The default is to pass --no-tags to clone, this option, if given,
        /// will cause git to do its normal default tag following behavior
        #[clap(long)]
        follow_tags: bool,

        /// If not given, we use --filter=blob:none. To use a different filter
        /// argument, use this option. To disable filtering, use --no-filter.
        #[clap(long, default_value = "blob:none")]
        filter: String,

        /// Do not pass a filter flag to git-clone. If both --no-filter and --filter
        /// options are given, --no-filter wins
        #[clap(long)]
        no_filter: bool,

        #[clap(long)]
        bare: bool,

        #[clap(long)]
        sparse: bool,

        #[clap(long)]
        progress: bool,

        #[clap(long)]
        push_url: Option<String>,

        #[clap(long, default_value=init::SOURCE_RO_URL)]
        fetch_url: String,

        #[clap()]
        target_path: String,
    },

    UserInterfaceTest {},

    #[clap(hide = true)]
    Maintenance {
        /// The git config key to look for paths of repos to run maintenance in. Defaults to
        /// 'maintenance.repo'
        #[clap(long, default_value=maintenance::DEFAULT_CONFIG_KEY, global = true)]
        config_key: String,

        #[clap(subcommand)]
        subcommand: MaintenanceSubcommand,
    },
}

#[derive(Parser, Debug)]
enum MaintenanceSubcommand {
    /// Runs global (i.e. system-wide) git maintenance tasks on repositories listed in
    /// the $HOME/.gitconfig's `maintenance.repo` multi-value config key. This command
    /// is usually run by a system-specific scheduler (eg. launchd) so it's unlikely that
    /// end users would need to invoke this command directly.
    Run {
        /// The absolute path to the git binary to use. If not given, the first 'git' in PATH
        /// will be used.
        #[clap(long)]
        git_binary_path: Option<PathBuf>,

        /// The absolute path to the git 'libexec' directory to use. If not given, the output of
        /// `git --exec-path` will be used.
        #[clap(long)]
        exec_path: Option<PathBuf>,

        /// The git config file to use to read the list of repos to run maintenance in. If not
        /// given, then use the default 'global' config which is usually $HOME/.gitconfig.
        #[clap(long)]
        config_path: Option<PathBuf>,

        /// The time period of job to run
        #[clap(
            long,
            possible_values=TimePeriod::VARIANTS,
            default_value="hourly",
        )]
        time_period: TimePeriod,
    },

    Register {
        #[clap(long, parse(from_os_str))]
        repo_path: Option<PathBuf>,

        #[clap(long, parse(from_os_str))]
        config_path: Option<PathBuf>,
    },

    Schedule {
        #[clap(long, parse(from_os_str), default_value = "~/Library/LaunchAgents")]
        launch_agents_path: PathBuf,

        /// The time period of job to schedule
        #[clap(
            long,
            possible_values=TimePeriod::VARIANTS,
            default_value="hourly",
        )]
        time_period: TimePeriod,

        /// register jobs for all time periods
        #[clap(long, conflicts_with = "time-period")]
        all: bool,
    },
}

#[derive(Parser, Debug)]
enum RepoSubcommand {
    /// List registered repositories
    List {},

    /// Attempt to repair the registry of repositories
    Repair {},
}

#[derive(Parser, Debug)]
enum LayerSubcommand {
    /// List all available layers
    Available {},

    /// List currently selected layers
    List {},

    /// Push a layer onto the top of the stack of currently selected layers
    Push {
        /// Names of layers to push.
        names: Vec<String>,
    },

    /// Pop one or more layer(s) from the top of the stack of current selected layers
    Pop {
        /// The number of layers to pop.
        #[clap(long, default_value = "1")]
        count: usize,
    },

    /// Filter out one or more layer(s) from the stack of currently selected layers
    Remove {
        /// Names of the layers to be removed.
        names: Vec<String>,
    },
}

#[derive(Parser, Debug)]
enum AdhocSubcommand {
    /// List the contents of the ad-hoc coordinate stack
    List {},

    /// Push one or more coordinate(s) onto the top of the ad-hoc coordinate stack
    Push {
        /// Names of coordinates to push.
        names: Vec<String>,
    },

    /// Pop one or more coordinates(s) from the top of the ad-hoc coordinate stack
    Pop {
        /// The number of coordinates to pop.
        #[clap(long, default_value = "1")]
        count: usize,
    },

    /// Filter out one or more coordinate(s) from the ad-hoc coordinate stack
    Remove {
        /// Names of the coordinates to be removed.
        names: Vec<String>,
    },
}

#[derive(Parser, Debug)]
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

#[derive(Parser, Debug)]
#[structopt(about = "Focused Development Tools")]
struct FocusOpts {
    /// Preserve the created sandbox directory for inspecting logs and other files.
    #[clap(long, global = true)]
    preserve_sandbox: bool,

    /// Disable textual user interface; happens by default on non-interactive terminals.
    #[clap(long, global = true)]
    ugly: bool,

    /// Number of threads to use when performing parallel resolution (where possible).
    #[clap(long, default_value = "0", global = true)]
    resolution_threads: usize,

    /// Change to the provided directory before doing anything else.
    #[clap(short = 'C', long = "work-dir", global = true)]
    working_directory: Option<PathBuf>,

    #[clap(subcommand)]
    cmd: Subcommand,
}

fn ensure_directories_exist() -> Result<()> {
    Tracker::default()
        .ensure_directories_exist()
        .context("creating directories for the tracker")?;

    Ok(())
}

fn expand_tilde<P: AsRef<Path>>(path_user_input: P) -> Result<PathBuf> {
    let p = path_user_input.as_ref();
    if !p.starts_with("~") {
        return Ok(p.to_path_buf());
    }
    if p == Path::new("~") {
        if let Some(home_dir) = dirs::home_dir() {
            return Ok(home_dir);
        } else {
            bail!("Could not determine home directory");
        }
    }

    let result = dirs::home_dir().map(|mut h| {
        if h == Path::new("/") {
            // Corner case: `h` root directory;
            // don't prepend extra `/`, just drop the tilde.
            p.strip_prefix("~").unwrap().to_path_buf()
        } else {
            h.push(p.strip_prefix("~/").unwrap());
            h
        }
    });

    if let Some(path) = result {
        Ok(path)
    } else {
        bail!("Failed to expand tildes in path '{}'", p.display());
    }
}

fn path_has_ancestor(subject: &Path, ancestor: &Path) -> Result<bool> {
    if subject == ancestor {
        return Ok(true);
    }

    let mut subject = subject;
    while let Some(parent) = subject.parent() {
        if parent == ancestor {
            return Ok(true);
        }

        subject = parent;
    }

    Ok(false)
}

fn run_subcommand(app: Arc<App>, options: FocusOpts, interactive: bool) -> Result<()> {
    let cloned_app = app.clone();

    match options.cmd {
        Subcommand::Clone {
            single_branch,
            dense_repo,
            sparse_repo,
            branch,
            coordinates_and_layers,
        } => {
            let ui = cloned_app.ui();
            let dense_repo =
                expand_tilde(dense_repo).context("Failed to expand dense repo path")?;
            let sparse_repo = {
                let current_dir =
                    env::current_dir().context("Failed to obtain current directory")?;
                let expanded =
                    expand_tilde(sparse_repo).context("Failed to expand sparse repo path")?;
                current_dir.join(expanded)
            };
            ui.log(
                "Clone",
                format!("Using the dense repo in {}", dense_repo.display()),
            );

            if path_has_ancestor(&sparse_repo, &dense_repo)
                .context("Could not determine if the sparse repo is in the dense repo")?
            {
                bail!("The sparse repo ({}) must not be be inside the dense repo ({}). Note: the sparse repo path is treated as relative to the current directory.", sparse_repo.display(), dense_repo.display())
            }

            let dense_repo = git_helper::find_top_level(cloned_app.clone(), &dense_repo)
                .context("Failed to canonicalize dense repo path")?;

            let (coordinates, layers): (Vec<String>, Vec<String>) = coordinates_and_layers
                .into_iter()
                .partition(|item| Coordinate::try_from(item.as_str()).is_ok());

            ui.status(format!(
                "Cloning {} into {}",
                dense_repo.display(),
                sparse_repo.display()
            ));
            ui.set_enabled(interactive);

            subcommands::clone::run(
                dense_repo,
                sparse_repo,
                branch,
                coordinates,
                layers,
                !single_branch,
                cloned_app,
            )
        }

        Subcommand::Sync { sparse_repo } => {
            let sparse_repo = expand_tilde(sparse_repo)?;
            app.ui().set_enabled(interactive);
            subcommands::sync::run(app, &sparse_repo)
        }

        Subcommand::Refs {
            repo: repo_path,
            subcommand,
        } => {
            let repo = Repository::open(repo_path).context("opening the repo")?;
            match subcommand {
                RefsSubcommand::Delete {
                    cutoff_date,
                    use_transaction,
                    check_merge_base,
                } => {
                    let cutoff = FocusTime::parse_date(cutoff_date)?;
                    app.ui().set_enabled(interactive);
                    refs::expire_old_refs(&repo, cutoff, check_merge_base, use_transaction, app)
                }

                RefsSubcommand::ListExpired {
                    cutoff_date,
                    check_merge_base,
                } => {
                    let cutoff = FocusTime::parse_date(cutoff_date)?;
                    let refs::PartitionedRefNames {
                        current: _,
                        expired,
                    } = refs::PartitionedRefNames::for_repo(&repo, cutoff, check_merge_base)?;

                    println!("{}", expired.join("\n"));

                    Ok(())
                }

                RefsSubcommand::ListCurrent {
                    cutoff_date,
                    check_merge_base,
                } => {
                    let cutoff = FocusTime::parse_date(cutoff_date)?;
                    let refs::PartitionedRefNames {
                        current,
                        expired: _,
                    } = refs::PartitionedRefNames::for_repo(&repo, cutoff, check_merge_base)?;

                    println!("{}", current.join("\n"));

                    Ok(())
                }
            }
        }

        Subcommand::Repo { subcommand } => match subcommand {
            RepoSubcommand::List {} => subcommands::repo::list(),
            RepoSubcommand::Repair {} => subcommands::repo::repair(app),
        },

        Subcommand::DetectBuildGraphChanges { repo } => {
            let repo = expand_tilde(repo)?;
            let repo = git_helper::find_top_level(app.clone(), &repo)
                .context("Failed to canonicalize repo path")?;
            subcommands::detect_build_graph_changes::run(app, &repo)
        }

        Subcommand::UserInterfaceTest {} => {
            let ui = cloned_app.ui();
            ui.status("UI Test");
            ui.set_enabled(interactive);
            subcommands::user_interface_test::run(app)
        }

        Subcommand::Layer { repo, subcommand } => {
            paths::assert_focused_repo(&repo)?;

            let should_check_tree_cleanliness = match subcommand {
                LayerSubcommand::Available {} => false,
                LayerSubcommand::List {} => false,
                LayerSubcommand::Push { names: _ } => true,
                LayerSubcommand::Pop { count: _ } => true,
                LayerSubcommand::Remove { names: _ } => true,
            };
            if should_check_tree_cleanliness {
                subcommands::sync::ensure_working_trees_are_clean(
                    app.clone(),
                    repo.as_path(),
                    None,
                )
                .context("Ensuring working trees are clean failed")?;
            }

            let selected_layer_stack_backup = {
                let sets = LayerSets::new(&repo);
                if sets.selected_layer_stack_path().is_file() {
                    Some(BackedUpFile::new(
                        sets.selected_layer_stack_path().as_path(),
                    )?)
                } else {
                    None
                }
            };

            let mutated = match subcommand {
                LayerSubcommand::Available {} => layer::available(&repo)?,
                LayerSubcommand::List {} => layer::list(&repo)?,
                LayerSubcommand::Push { names } => layer::push(&repo, names)?,
                LayerSubcommand::Pop { count } => layer::pop(&repo, count)?,
                LayerSubcommand::Remove { names } => layer::remove(&repo, names)?,
            };

            if mutated {
                app.ui().log(
                    "Layer Stack Update",
                    "Syncing focused paths since the selected content has changed",
                );
                app.ui().set_enabled(interactive);
                subcommands::sync::run(app, repo.as_path())
                    .context("Sync failed; changes to the stack will be reverted.")?;
            }

            // If there was a change, the sync succeeded, so we we can discard the backup.
            if let Some(backup) = selected_layer_stack_backup {
                backup.set_restore(false);
            }

            Ok(())
        }

        Subcommand::Adhoc { repo, subcommand } => {
            paths::assert_focused_repo(&repo)?;

            let should_check_tree_cleanliness = match subcommand {
                AdhocSubcommand::List {} => false,
                AdhocSubcommand::Push { names: _ } => true,
                AdhocSubcommand::Pop { count: _ } => true,
                AdhocSubcommand::Remove { names: _ } => true,
            };
            if should_check_tree_cleanliness {
                subcommands::sync::ensure_working_trees_are_clean(
                    app.clone(),
                    repo.as_path(),
                    None,
                )
                .context("Ensuring working trees are clean failed")?;
            }

            let adhoc_layer_set_backup = {
                let sets = LayerSets::new(&repo);
                if sets.adhoc_layer_path().is_file() {
                    Some(BackedUpFile::new(sets.adhoc_layer_path().as_path())?)
                } else {
                    None
                }
            };

            let mutated: bool = match subcommand {
                AdhocSubcommand::List {} => adhoc::list(app.clone(), repo.clone())?,
                AdhocSubcommand::Push { names } => adhoc::push(app.clone(), repo.clone(), names)?,
                AdhocSubcommand::Pop { count } => adhoc::pop(app.clone(), repo.clone(), count)?,
                AdhocSubcommand::Remove { names } => {
                    adhoc::remove(app.clone(), repo.clone(), names)?
                }
            };

            if mutated {
                app.ui().log(
                    "Ad-hoc Coordinate Stack",
                    "Syncing focused paths since the selected content has changed",
                );
                app.ui().set_enabled(interactive);
                subcommands::sync::run(app, repo.as_path())
                    .context("Sync failed; changes to the stack will be reverted.")?;
            }

            // Sync (if necessary) succeeded, so skip reverting the ad-hoc coordinate stack.
            if let Some(backup) = adhoc_layer_set_backup {
                backup.set_restore(false);
            }

            Ok(())
        }

        Subcommand::Init {
            shallow_since,
            branch_name,
            no_checkout,
            follow_tags,
            filter,
            no_filter,
            bare,
            sparse,
            progress,
            fetch_url,
            push_url,
            target_path,
        } => {
            let ui = app.ui();
            ui.status("Init repo");

            let expanded =
                expand_tilde(target_path).context("expanding tilde on target_path argument")?;

            let target = expanded.as_path();

            let mut init_opts: Vec<InitOpt> = Vec::new();

            let mut add_if_true = |n: bool, opt: InitOpt| {
                if n {
                    init_opts.push(opt)
                };
            };

            add_if_true(no_checkout, InitOpt::NoCheckout);
            add_if_true(bare, InitOpt::Bare);
            add_if_true(sparse, InitOpt::Sparse);
            add_if_true(follow_tags, InitOpt::FollowTags);
            add_if_true(progress, InitOpt::Progress);

            ui.set_enabled(interactive);

            ui.log(
                "Init",
                format!("setting up a copy of the repo in {:?}", target),
            );

            init::run(
                shallow_since,
                Some(branch_name),
                if no_filter { None } else { Some(filter) },
                fetch_url,
                push_url,
                target.to_owned(),
                init_opts,
                app,
            )?;

            Ok(())
        }

        Subcommand::Maintenance {
            subcommand,
            config_key,
        } => match subcommand {
            MaintenanceSubcommand::Run {
                git_binary_path,
                exec_path,
                config_path,
                time_period,
            } => maintenance::run(
                maintenance::RunOptions {
                    git_binary_path,
                    config_key,
                    exec_path,
                    config_path,
                },
                time_period,
            ),
            MaintenanceSubcommand::Register {
                repo_path,
                config_path,
            } => maintenance::register(maintenance::RegisterOpts {
                repo_path,
                config_key,
                global_config_path: config_path,
            }),
            MaintenanceSubcommand::Schedule {
                launch_agents_path,
                time_period,
                all,
            } => {
                let time_periods: Vec<TimePeriod> = if all {
                    maintenance::TimePeriod::iter().collect()
                } else {
                    vec![time_period]
                };

                for tp in time_periods {
                    maintenance::write_plist(
                        launchd::PlistOpts::default(),
                        tp,
                        &expand_tilde(&launch_agents_path)?,
                    )?;
                }

                todo!("implement calling launchctl with new jobs");
            }
        },
    }
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

fn main() -> Result<()> {
    let started_at = Instant::now();
    let options = FocusOpts::parse();
    if let Some(working_directory) = &options.working_directory {
        std::env::set_current_dir(working_directory).context("Switching working directory")?;
    }
    setup_thread_pool(options.resolution_threads)?;

    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let interactive = if options.ugly {
        false
    } else {
        termion::is_tty(&std::io::stdout())
    };

    ensure_directories_exist().context("Failed to create necessary directories")?;
    let app = Arc::from(App::new(options.preserve_sandbox, interactive)?);
    run_subcommand(app, options, interactive)?;

    let total_runtime = started_at.elapsed();
    log::debug!("Finished normally in {:.2}s", total_runtime.as_secs_f32());

    Ok(())
}
