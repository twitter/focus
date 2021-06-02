use anyhow::Result;
use env_logger::{self, Env};
use internals::error::AppError;
use std::path::PathBuf;
use structopt::StructOpt;

// Servers are per-repo
// They communicate through a domain socket
//   $GIT_DIR/store/socket
// Database is stored in $GIT_DIR/store/database
// Eccentric BS: alternates / "shared" repos

mod endpoint;

#[derive(StructOpt, Debug)]
enum Subcommand {
    Run {
        #[structopt(long, default_value = "repo", parse(from_os_str))]
        repo_path: PathBuf,
    },
}

#[derive(StructOpt, Debug)]
#[structopt(about = "Project-focused development client")]
struct RepoManagerOpts {
    #[structopt(subcommand)]
    cmd: Subcommand,
}

fn main() -> Result<(), AppError> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let opt = RepoManagerOpts::from_args();
    match opt.cmd {
        Subcommand::Run { repo_path: _ } => Ok(()),
    }
}
