mod detail;
mod manager;
mod model;
mod tracker;
mod sandbox;
mod sandbox_command;
mod sparse_repos;
mod subcommands;
mod temporary_working_directory;
mod testing;
mod working_tree_synchronizer;
mod git_helper;
#[macro_use]
extern crate lazy_static;

use anyhow::{bail, Context, Result};
use env_logger::{self, Env};

use sandbox::Sandbox;

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
    Clone {
        #[structopt(long, parse(from_os_str))]
        dense_repo: PathBuf,

        #[structopt(long, parse(from_os_str))]
        sparse_repo: PathBuf,

        #[structopt(long)]
        branch: String,

        #[structopt(long, default_value = "")]
        coordinates: CommaSeparatedStrings,

        #[structopt(long, default_value = "")]
        layers: CommaSeparatedStrings,

        #[structopt(long)]
        filter_sparse: bool,

        #[structopt(long)]
        generate_project_view: bool,
    },

    Sync {
        #[structopt(long, parse(from_os_str), default_value = ".dense")]
        dense_repo: PathBuf,

        #[structopt(long, parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,
    },

    Invalidate {
        #[structopt(long, parse(from_os_str), default_value = ".dense")]
        dense_repo: PathBuf,

        #[structopt(long, parse(from_os_str), default_value = ".")]
        sparse_repo: PathBuf,
    },

    AvailableLayers {
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,
    },

    SelectedLayers {
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,
    },

    PushLayer {
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        names: Vec<String>,
    },

    PopLayer {
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        #[structopt(long, default_value = "1")]
        count: usize,
    },

    RemoveLayer {
        #[structopt(long, parse(from_os_str), default_value = ".")]
        repo: PathBuf,

        names: Vec<String>,
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

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

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

        Subcommand::Invalidate {
            dense_repo,
            sparse_repo,
        } => subcommands::invalidate::run(&sandbox, &dense_repo, &sparse_repo),

        Subcommand::Sync {
            dense_repo,
            sparse_repo,
        } => subcommands::sync::run(&sandbox, &dense_repo, &sparse_repo),

        Subcommand::AvailableLayers { repo } => subcommands::available_layers::run(&repo),

        Subcommand::SelectedLayers { repo } => subcommands::selected_layers::run(&repo),

        Subcommand::PushLayer { repo, names } => subcommands::push_layer::run(&repo, names),

        Subcommand::PopLayer { repo, count } => subcommands::pop_layer::run(&repo, count),

        Subcommand::RemoveLayer { repo, names } => subcommands::remove_layer::run(&repo, names),
    }
}
