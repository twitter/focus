use anyhow::Result;
use env_logger::{self, Env};
use internals::{error::AppError, repo::Repos};
use log::{debug, error, info};
use serde::__private::ser;
use std::{path::PathBuf, sync::Arc};
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
enum Subcommand {
    Import {},
    List {},
    Run {
        #[structopt(
            long,
            default_value = "target/release/libfocus_redis_repo_server.so",
            parse(from_os_str)
        )]
        server_module_path: PathBuf,
    },
    Connect {},
}

#[derive(StructOpt, Debug)]
#[structopt(about = "Project-focused development client")]
struct RepoManagerOpts {
    #[structopt(subcommand)]
    cmd: Subcommand,
}

fn import() -> Result<(), AppError> {
    let repos = Repos::new(None)?;
    if let Ok(locked_repos) = repos.underlying.lock() {
        for (k, arc_mutex_repo) in locked_repos.iter() {
            if let Ok(locked_repo) = arc_mutex_repo.lock() {
                locked_repo.import()?
            } else {
                error!("Locking for read failed")
            }
        }
    }

    return Ok(());
}

fn list_repos() -> Result<(), AppError> {
    let repos = Repos::new(None)?;
    if let Ok(locked_repos) = repos.underlying.lock() {
        for (k, arc_mutex_repo) in locked_repos.iter() {
            if let Ok(locked_repo) = arc_mutex_repo.lock() {
                if let Some(dir) = locked_repo.work_dir()? {
                    println!("{} {}", &k, dir)
                }
            } else {
                error!("Locking for read failed")
            }
        }
    }

    Ok(())
}

mod redis {
    use internals::error::AppError;
    use log::info;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    pub(crate) fn run_server(server_module_path: PathBuf) -> Result<(), AppError> {
        std::fs::metadata(&server_module_path).expect("Server module cannot be accessed");
        let config_path = write_redis_config()?;
        info!("Starting redis-server (socket: {:?})", redis_socket_path()?);
        Command::new("redis-server")
            .arg(config_path)
            .arg("--loadmodule")
            .arg(&server_module_path.to_str()?)
            .spawn()?
            .wait()
            .expect("redis-server failed");
        Ok(())
    }

    pub(crate) fn connect_server() -> Result<(), AppError> {
        info!("Starting redis-cli");
        Command::new("redis-cli")
            .arg("-s")
            .arg(&redis_socket_path()?.to_string_lossy()[..])
            .spawn()?
            .wait()
            .expect("redis-cli failed");
        Ok(())
    }

    fn redis_dir() -> PathBuf {
        internals::config::fs::data_dir().join("redis")
    }

    fn redis_config_path() -> Result<PathBuf, AppError> {
        Ok(redis_dir().join("redis.conf"))
    }

    fn redis_socket_path() -> Result<PathBuf, AppError> {
        Ok(redis_dir().join("socket"))
    }

    fn write_redis_config() -> Result<PathBuf, AppError> {
        use std::io::Write;

        let dir = redis_dir();
        fs::create_dir_all(&dir)?;
        let config_path = redis_config_path()?;
        let mut file = fs::File::create(&config_path)?;
        writeln!(file, "port 0")?;
        writeln!(file, "unixsocket '{}'", redis_socket_path()?.display())?;
        writeln!(file, "timeout 0")?;
        writeln!(file, "daemonize no")?;
        writeln!(file, "databases 8")?;
        writeln!(file, "appendonly no")?;
        writeln!(file, "pidfile '{}/server.pid'", dir.display())?;
        writeln!(file, "loglevel debug")?;
        writeln!(file, "logfile '{}/server.log'", dir.display())?;
        // writeln!(file, "set-proc-title yes")?;
        // writeln!(
        //     file,
        //     "proc-title-template \"{}\"",
        //     "ee_scm_{title} {config-file} {unixsocket} {server-mode}",
        // )?;
        Ok(config_path)
    }
}

fn main() -> Result<(), AppError> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let opt = RepoManagerOpts::from_args();
    match opt.cmd {
        Subcommand::Import {} => return import(),
        Subcommand::List {} => return list_repos(),
        Subcommand::Run { server_module_path } => return redis::run_server(server_module_path),
        Subcommand::Connect {} => return redis::connect_server(),
        _ => {
            error!("unsupported command");
            Err(AppError::InvalidArgs())
        }
    }
}
