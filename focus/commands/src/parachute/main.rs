mod client;
mod detail;
mod testing;
mod util;
mod working_tree_synchronizer;

#[macro_use]
extern crate lazy_static;

use anyhow::{bail, Result};
use env_logger::{self, Env};
use focus_formats::parachute::Coordinate;
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

// #[derive(Debug, StructOpt)]
// pub struct CoordinateOpt {
//     #[structopt(help = "Build coordinate")]
//     underlying: Coordinates,
// }

#[derive(StructOpt, Debug)]
enum Subcommand {
    Server {
        #[structopt(long, parse(from_os_str))]
        root: PathBuf,

        #[structopt(long, parse(from_os_str))]
        data: PathBuf,
    },

    Client {
        #[structopt(long, parse(from_os_str))]
        source: PathBuf,

        #[structopt(long, parse(from_os_str))]
        target: PathBuf,

        #[structopt(long, help = "Comma-separated list of build coordinates")]
        coordinates: Coordinates,
    },
}

#[derive(StructOpt, Debug)]
#[structopt(about = "coordinates Focused Development Client")]
struct ParachuteOpts {
    #[structopt(subcommand)]
    cmd: Subcommand,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let opt = ParachuteOpts::from_args();
    match opt.cmd {
        // Subcommand::Server { root, data } => return detail::server(root.as_path(), data.as_path()),
        Subcommand::Client {
            source,
            target,
            coordinates,
        } => {
            let coordinates = coordinates.0;
            for coord in &coordinates {
                log::info!("coord:{}", coord);
            }
            client::run_client(source.as_path(), target.as_path(), coordinates)?;
            // }
            Ok(())
        }
        _ => {
            bail!("Not implemented");
        }
    }
}
