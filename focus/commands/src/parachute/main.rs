mod sparse_repos;
mod detail;
mod testing;
mod util;
mod working_tree_synchronizer;

#[macro_use]
extern crate lazy_static;

use anyhow::{bail, Result};
use env_logger::{self, Env};
use log::error;
use std::{path::PathBuf, str::FromStr};
use structopt::StructOpt;

#[derive(Debug)]
struct Coordinates(Vec<String>);

impl FromStr for Coordinates {
    type Err = std::string::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let splayed: Vec<_> = s.split(",").map(|s| s.to_owned()).collect();
        Ok(Coordinates(splayed))
    }
}

#[derive(StructOpt, Debug)]
enum Subcommand {
    GenerateSparseProfile {
        #[structopt(long, parse(from_os_str))]
        dense_repo: PathBuf,

        #[structopt(long, parse(from_os_str))]
        sparse_profile_output: PathBuf,

        #[structopt(long)]
        coordinates: Coordinates,
    },

    CreateSparseClone {
        #[structopt(long)]
        name: String,

        #[structopt(long, parse(from_os_str))]
        dense_repo: PathBuf,

        #[structopt(long, parse(from_os_str))]
        sparse_repo: PathBuf,

        #[structopt(long)]
        branch: String,

        #[structopt(long)]
        coordinates: Coordinates,
    },
}

#[derive(StructOpt, Debug)]
#[structopt(about = "Focused Development Tools")]
struct ParachuteOpts {
    #[structopt(subcommand)]
    cmd: Subcommand,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let opt = ParachuteOpts::from_args();
    match opt.cmd {
        Subcommand::CreateSparseClone {
            name,
            dense_repo,
            sparse_repo,
            branch,
            coordinates,
        } => {
            sparse_repos::create_sparse_clone(&name, &dense_repo, &sparse_repo, &branch, &coordinates.0)?;
            Ok(())
        }

        Subcommand::GenerateSparseProfile {
            dense_repo,
            sparse_profile_output,
            coordinates,
        } => {
            sparse_repos::generate_sparse_profile(&dense_repo, &sparse_profile_output, &coordinates.0)?;
            Ok(())
        }

        _ => {
            bail!("Not implemented");
        }
    }
}
