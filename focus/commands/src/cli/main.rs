mod backed_up_file;
mod detail;
mod git_helper;
mod model;
mod sandbox;
mod sandbox_command;
mod sparse_repos;
mod subcommands;
mod temporary_working_directory;
mod testing;
mod tracker;
mod working_tree_synchronizer;

#[macro_use]
extern crate lazy_static;

use anyhow::{bail, Context, Result};
use env_logger::{self, Env};

use sandbox::Sandbox;
use tracker::Tracker;

use std::{path::PathBuf, str::FromStr, sync::Arc};
use structopt::StructOpt;

#[derive(Debug)]
struct CommaSeparatedStrings(Vec<String>);

impl FromStr for CommaSeparatedStrings {
    type Err = std::string::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let splayed: Vec<_> = s.split(",").map(|s| s.to_owned()).collect();
        Ok(CommaSeparatedStrings(splayed))
    }
}

#[derive(StructOpt, Debug)]
enum Subcommand {
    /// Create a sparse clone from named layers or ad-hoc build coordinates
    Clone {
        /// Path to the existing dense repository that the sparse clone shall be based upon.
        #[structopt(long, parse(from_os_str))]
        dense_repo: PathBuf,

        /// Path where the new sparse repository should be created.
        #[structopt(long, parse(from_os_str))]
        sparse_repo: PathBuf,

        /// The name of the branch to clone.
        #[structopt(long)]
        branch: String,

        /// Bazel build coordinates to include as an ad-hoc layer set, cannot be specified in combination with 'layers'.
        #[structopt(long, default_value = "")]
        coordinates: CommaSeparatedStrings,

        /// Named layers to include. Comma separated, loaded from the dense repository's `focus/projects` directory), cannot be specified in combination with 'coordinates'.
        #[structopt(long, default_value = "")]
        layers: CommaSeparatedStrings,

        /// Experimental, NOT RECOMMENDED: use sparse filtering (`--filter:oid:...`) when cloning.
        #[structopt(long)]
        filter_sparse: bool,

        /// When specified a Bazel project view will be written to `focus-<repo-name>.bazelproject` in the sparse repository.
        #[structopt(long)]
        generate_project_view: bool,
    },

    /// Update the sparse checkout to reflect changes to the build graph.
    Sync {
        /// Path to the dense repository. Build graph queries are always run in the dense repository.
        #[structopt(long, parse(from_os_str), default_value = ".dense")]
        dense_repo: PathBuf,

        /// Path to the sparse repository.
        #[structopt(parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,
    },

    /// List available layers
    AvailableLayers {
        /// Path to the repository.
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,
    },

    /// List currently selected layers
    SelectedLayers {
        /// Path to the repository.
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,
    },

    /// Push a layer onto the top of the stack of currently selected layers
    PushLayer {
        /// Path to the repository.
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        /// Names of layers to push.
        names: Vec<String>,
    },

    /// Pop one or more layer(s) from the top of the stack of current selected layers
    PopLayer {
        /// Path to the repository.
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        /// The number of layers to pop.
        #[structopt(long, default_value = "1")]
        count: usize,
    },

    /// Filter out one or more layer(s) from the stack of currently selected layers
    RemoveLayer {
        /// Path to the repository.
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        /// Names of the layers to be removed.
        names: Vec<String>,
    },

    /// List focused repositories
    ListRepos {},

    /// Detect whether there are changes to the build graph (used internally)
    DetectBuildGraphChanges {
        // Path to the repository.
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,
    },
}

#[derive(StructOpt, Debug)]
#[structopt(about = "Focused Development Tools")]
struct ParachuteOpts {
    #[structopt(long)]
    preserve_sandbox: bool,

    #[structopt(subcommand)]
    cmd: Subcommand,
}

fn filter_empty_strings(string_list: Vec<String>) -> Vec<String> {
    string_list
        .iter()
        .filter_map(|s| {
            if !s.is_empty() {
                Some(s.to_owned())
            } else {
                None
            }
        })
        .collect()
}

fn ensure_directories_exist() -> Result<()> {
    Tracker::default()
        .ensure_directories_exist()
        .context("creating directories for the tracker")?;

    Ok(())
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    ensure_directories_exist().context("ensuring directories exist")?;

    let opt = ParachuteOpts::from_args();

    let sandbox = Arc::new(Sandbox::new(opt.preserve_sandbox).context("Creating a sandbox")?);

    match opt.cmd {
        Subcommand::Clone {
            dense_repo,
            sparse_repo,
            branch,
            layers,
            coordinates,
            filter_sparse,
            generate_project_view,
        } => {
            let layers = filter_empty_strings(layers.0);
            let coordinates = filter_empty_strings(coordinates.0);

            if !(coordinates.is_empty() ^ layers.is_empty()) {
                bail!("Either layers or coordinates must be specified");
            }

            let spec = if !coordinates.is_empty() {
                sparse_repos::Spec::Coordinates(coordinates.to_vec())
            } else if !layers.is_empty() {
                sparse_repos::Spec::Layers(layers.to_vec())
            } else {
                unreachable!()
            };

            subcommands::clone::run(
                &dense_repo,
                &sparse_repo,
                &branch,
                &spec,
                filter_sparse,
                generate_project_view,
                sandbox,
            )
        }

        Subcommand::Sync {
            dense_repo,
            sparse_repo,
        } => subcommands::sync::run(&sandbox, &dense_repo, &sparse_repo),

        Subcommand::AvailableLayers { repo } => subcommands::available_layers::run(&repo),

        Subcommand::SelectedLayers { repo } => subcommands::selected_layers::run(&repo),

        Subcommand::PushLayer { repo, names } => subcommands::push_layer::run(&repo, names),

        Subcommand::PopLayer { repo, count } => subcommands::pop_layer::run(&repo, count),

        Subcommand::RemoveLayer { repo, names } => subcommands::remove_layer::run(&repo, names),

        Subcommand::ListRepos {} => subcommands::list_repos::run(),

        Subcommand::DetectBuildGraphChanges { repo } => {
            subcommands::detect_build_graph_changes::run(&sandbox, &repo)
        }
    }
}
