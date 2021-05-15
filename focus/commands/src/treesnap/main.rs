#[macro_use]
extern crate lazy_static;

mod detail;

use anyhow::Result;
use env_logger::{self, Env};
use focus_formats::FormatsRoot;
use internals::error::AppError;
use log::{debug, error, info};
use std::path::{Path, PathBuf};
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
enum Subcommand {
    Snapshot {
        #[structopt(long, parse(from_os_str))]
        repo: PathBuf,
        #[structopt(long, parse(from_os_str))]
        output: PathBuf,
    },
    Difference {
        #[structopt(long, parse(from_os_str))]
        from_snapshot: PathBuf,
        #[structopt(long, parse(from_os_str))]
        to_snapshot: PathBuf,
        #[structopt(long, parse(from_os_str))]
        output: PathBuf,
    },
}

#[derive(StructOpt, Debug)]
#[structopt(about = "Focus SCM tree snap")]
struct TreesnapOpts {
    #[structopt(subcommand)]
    cmd: Subcommand,
}

fn main() -> Result<(), AppError> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let opt = TreesnapOpts::from_args();
    match opt.cmd {
        Subcommand::Snapshot { repo, output } => {
            return detail::snapshot(repo.as_path(), output.as_path())
        }
        Subcommand::Difference {
            from_snapshot,
            to_snapshot,
            output,
        } => return detail::difference(from_snapshot, to_snapshot, output),
        _ => {
            error!("unsupported command");
            Err(AppError::InvalidArgs())
        }
    }
}
