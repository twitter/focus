mod app;
mod coordinate;
mod coordinate_resolver;
mod model;
mod sparse_repos;
mod subcommands;
mod testing;
mod tracker;
mod ui;
mod util;
mod working_tree_synchronizer;

use anyhow::{bail, Context, Result};
use env_logger::{self, Env};

use tracker::Tracker;
use util::git_helper;

use std::{
    convert::TryFrom,
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use structopt::StructOpt;

use crate::{
    app::App,
    coordinate::Coordinate,
    model::LayerSets,
    subcommands::{adhoc, layer},
    util::{backed_up_file::BackedUpFile, repo_paths},
};

fn the_name_of_this_binary() -> String {
    std::env::args_os()
        .next()
        .unwrap_or(OsString::from("focus"))
        .to_str()
        .unwrap()
        .to_owned()
}

#[derive(StructOpt, Debug)]
enum Subcommand {
    /// Create a sparse clone from named layers or ad-hoc build coordinates
    Clone {
        /// Path to the dense repository to base the clone on.
        #[structopt(long, parse(from_os_str), default_value = "~/workspace/source")]
        dense_repo: PathBuf,

        /// Path where the new sparse repository should be created.
        #[structopt(parse(from_os_str))]
        sparse_repo: PathBuf,

        /// The name of the branch to clone.
        #[structopt(long, default_value = "master")]
        branch: String,

        /// Named layers and ad-hoc coordinates to include in the clone. Named layers are loaded from the dense repo's `focus/projects` directory.
        coordinates_and_layers: Vec<String>,
    },

    /// Update the sparse checkout to reflect changes to the build graph.
    Sync {
        /// Path to the sparse repository.
        #[structopt(parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,
    },

    /// Interact with repos configured on this system. Run `focus repo help` for more information.
    Repo {
        args: Vec<String>,
    },

    /// Interact with the stack of selected layers. Run `focus layer help` for more information.
    Layer {
        /// Path to the repository.
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        args: Vec<String>,
    },

    /// Interact with the ad-hoc coordinate stack. Run `focus adhoc help` for more information.
    Adhoc {
        /// Path to the repository.
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        args: Vec<String>,
    },

    /// Detect whether there are changes to the build graph (used internally)
    DetectBuildGraphChanges {
        // Path to the repository.
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,
    },

    UserInterfaceTest {},
}

#[derive(StructOpt, Debug)]
struct RepoSubcommand {
    #[structopt(subcommand)]
    verb: RepoOpts,
}

#[derive(StructOpt, Debug)]
enum RepoOpts {
    /// List registered repositories
    List {},

    /// Attempt to repair the registry of repositories
    Repair {},
}

#[derive(StructOpt, Debug)]
enum LayersOpts {
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
        #[structopt(long, default_value = "1")]
        count: usize,
    },

    /// Filter out one or more layer(s) from the stack of currently selected layers
    Remove {
        /// Names of the layers to be removed.
        names: Vec<String>,
    },
}

#[derive(StructOpt, Debug)]
struct LayerSubcommand {
    #[structopt(subcommand)]
    verb: LayersOpts,
}

#[derive(StructOpt, Debug)]
enum AdhocOpts {
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
        #[structopt(long, default_value = "1")]
        count: usize,
    },

    /// Filter out one or more coordinate(s) from the ad-hoc coordinate stack
    Remove {
        /// Names of the coordinates to be removed.
        names: Vec<String>,
    },
}

#[derive(StructOpt, Debug)]
struct AdhocSubcommand {
    #[structopt(subcommand)]
    verb: AdhocOpts,
}

#[derive(StructOpt, Debug)]
#[structopt(about = "Focused Development Tools")]
struct FocusOpts {
    /// Preserve the created sandbox directory for inspecting logs and other files.
    #[structopt(long)]
    preserve_sandbox: bool,

    /// Disable textual user interface; happens by default on non-interactive terminals.
    #[structopt(long)]
    ugly: bool,

    #[structopt(long, default_value = "1")]
    task_threads: usize,

    #[structopt(subcommand)]
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
                String::from("Clone"),
                format!("Using the dense repo in {}", dense_repo.display()),
            );

            if path_has_ancestor(&sparse_repo, &dense_repo)
                .context("Could not determine if the sparse repo is in the dense repo")?
            {
                bail!("The sparse repo ({}) must not be be inside the dense repo ({}). Note: the sparse repo path is treated as relative to the current directory.", sparse_repo.display(), dense_repo.display())
            }

            let dense_repo = git_helper::find_top_level(cloned_app.clone(), &dense_repo)
                .context("Failed to canonicalize dense repo path")?;

            let (coordinates, layers): (Vec<&String>, Vec<&String>) = coordinates_and_layers
                .iter()
                .partition(|&item| Coordinate::try_from(item.as_str()).is_ok());
            let coordinates = coordinates.iter().map(|&item| item.to_owned()).collect();
            let layers = layers.iter().map(|&item| item.to_owned()).collect();

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
                cloned_app.clone(),
            )
        }

        Subcommand::Sync { sparse_repo } => {
            let sparse_repo = expand_tilde(sparse_repo)?;
            app.ui().set_enabled(interactive);
            subcommands::sync::run(app, &sparse_repo)
        }

        Subcommand::Repo { args } => {
            // Note: This is hacky, but it allows us to have second-level subcommands, which structopt otherwise does not support.
            let mut args = args.clone();
            args.insert(0, format!("{} repo", the_name_of_this_binary()));
            let repo_subcommand = RepoSubcommand::from_iter(args.iter());
            match repo_subcommand.verb {
                RepoOpts::List {} => subcommands::repo::list(),
                RepoOpts::Repair {} => subcommands::repo::repair(app.clone()),
            }
        }
        Subcommand::DetectBuildGraphChanges { repo } => {
            let repo = expand_tilde(repo)?;
            let repo = git_helper::find_top_level(app.clone(), &repo)
                .context("Failed to canonicalize repo path")?;
            subcommands::detect_build_graph_changes::run(app, &repo)
        }

        Subcommand::UserInterfaceTest {} => {
            let ui = cloned_app.ui();
            ui.status(format!("UI Test"));
            ui.set_enabled(interactive);
            subcommands::user_interface_test::run(app)
        }

        Subcommand::Layer { repo, args } => {
            repo_paths::assert_focused_repo(&repo)?;

            // Note: This is hacky, but it allows us to have second-level subcommands, which structopt otherwise does not support.
            let mut args = args.clone();
            args.insert(0, format!("{} layer", the_name_of_this_binary()));
            let layer_subcommand = LayerSubcommand::from_iter(args.iter());

            let should_check_tree_cleanliness = match layer_subcommand.verb {
                LayersOpts::Available {} => false,
                LayersOpts::List {} => false,
                LayersOpts::Push { names: _ } => true,
                LayersOpts::Pop { count: _ } => true,
                LayersOpts::Remove { names: _ } => true,
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

            let mutated = match layer_subcommand.verb {
                LayersOpts::Available {} => layer::available(&repo)?,
                LayersOpts::List {} => layer::list(&repo)?,
                LayersOpts::Push { names } => layer::push(&repo, names)?,
                LayersOpts::Pop { count } => layer::pop(&repo, count)?,
                LayersOpts::Remove { names } => layer::remove(&repo, names)?,
            };

            if mutated {
                app.ui().log(
                    String::from("Layer Stack Update"),
                    String::from("Syncing focused paths since the selected content has changed"),
                );
                app.ui().set_enabled(interactive);
                subcommands::sync::run(app, repo.as_path())
                    .context("Sync failed; changes to the stack will be reverted.")?;
            }

            // If there was a change, the ssync succeded, so we we can discard the backup.
            selected_layer_stack_backup.map(|backup| backup.set_restore(false));

            Ok(())
        }

        Subcommand::Adhoc { repo, args } => {
            repo_paths::assert_focused_repo(&repo)?;

            let mut args = args.clone();
            args.insert(0, format!("{} adhoc", the_name_of_this_binary()));
            let adhoc_subcommand = AdhocSubcommand::from_iter(args.iter());

            let should_check_tree_cleanliness = match adhoc_subcommand.verb {
                AdhocOpts::List {} => false,
                AdhocOpts::Push { names: _ } => true,
                AdhocOpts::Pop { count: _ } => true,
                AdhocOpts::Remove { names: _ } => true,
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

            let mutated: bool = match adhoc_subcommand.verb {
                AdhocOpts::List {} => adhoc::list(app.clone(), repo.clone())?,
                AdhocOpts::Push { names } => adhoc::push(app.clone(), repo.clone(), names)?,
                AdhocOpts::Pop { count } => adhoc::pop(app.clone(), repo.clone(), count)?,
                AdhocOpts::Remove { names } => adhoc::remove(app.clone(), repo.clone(), names)?,
            };

            if mutated {
                app.ui().log(
                    String::from("Ad-hoc Coordinate Stack"),
                    String::from("Syncing focused paths since the selected content has changed"),
                );
                app.ui().set_enabled(interactive);
                subcommands::sync::run(app, repo.as_path())
                    .context("Sync failed; changes to the stack will be reverted.")?;
            }

            // Sync (if necessary) succeeded, so skip reverting the ad-hoc coordinate stack.
            adhoc_layer_set_backup.map(|backup| {
                backup.set_restore(false);
            });

            Ok(())
        }
    }
}

fn setup_thread_pool(task_threads: usize) -> Result<()> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(task_threads)
        .build_global()
        .context("Failed to create the task thread pool")?;
    Ok(())
}

fn main() -> Result<()> {
    let started_at = Instant::now();
    let options = FocusOpts::from_args();
    setup_thread_pool(options.task_threads)?;

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
