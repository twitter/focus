mod detail;
mod testing;

#[macro_use]
extern crate lazy_static;

use anyhow::Result;
use env_logger::{self, Env};
use internals::error::AppError;
use log::{debug, error, info};
use std::path::{Path, PathBuf};
use structopt::StructOpt;

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
        repo: PathBuf,

        #[structopt(long)]
        project: String,
    },
}

#[derive(StructOpt, Debug)]
#[structopt(about = "Project Focused Development Client")]
struct ParachuteOpts {
    #[structopt(subcommand)]
    cmd: Subcommand,
}

fn main() -> Result<(), AppError> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let opt = ParachuteOpts::from_args();
    match opt.cmd {
        Subcommand::Server { root, data } => {
            return detail::server(root.as_path(), data.as_path())
        }
        _ => {
            error!("unsupported command");
            Err(AppError::InvalidArgs())
        }
    }
}
