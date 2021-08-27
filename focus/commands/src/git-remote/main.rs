use anyhow::{bail, Context, Result};
use env_logger::{self, Env};
use std::{
    collections::HashMap,
    convert::TryInto,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use structopt::StructOpt;
use url::Url;

#[derive(StructOpt, Debug)]
#[structopt(about = "Focus Git Remote Helper")]
struct Flags {
    remote: String,

    url: Url,

    #[structopt(long, parse(from_os_str), default_value = ".")]
    sparse_repo: PathBuf,

    #[structopt(long, default_value = "info")]
    default_log_level: String,
}

impl Flags {
    fn validate(&self) -> Result<()> {
        if !self.url.scheme().eq_ignore_ascii_case("file") {
            bail!("url has unsupported scheme");
        }

        Ok(())
    }

    fn remote_path(&self) -> Result<PathBuf> {
        Ok(PathBuf::from(self.url.path()))
    }
}

// fn git_dir() -> Result<PathBuf> {
//     Ok(PathBuf::from(
//         &std::env::var("GIT_DIR").context("reading the GIT_DIR environment variable")?,
//     ))
// }

struct Helper {
    flags: Flags,
}

impl Helper {
    pub(crate) fn new(flags: Flags) -> Result<Self> {
        Ok(Self { flags })
    }

    pub(crate) fn current_branch(&self) -> Result<String> {
        let output = Command::new("git")
            .arg("branch")
            .arg("--show-current")
            .current_dir(&self.flags.sparse_repo)
            .output()
            .context("running git-branch")?;

        Ok(String::from_utf8(output.stdout)
            .context("interpreting output as UTF-8")?
            .trim()
            .to_owned())
    }

    pub(crate) fn fetch_and_update_refs_in_dense_repo(&self) -> Result<()> {
        let branch = self
            .current_branch()
            .context("getting the current branch")?;

        let opt = Flags::from_args();
        if !opt.url.scheme().eq_ignore_ascii_case("file") {
            bail!("This helper only supports the 'file' scheme");
        }
        let dense_path = PathBuf::from(opt.url.path());
        if !dense_path.is_dir() {
            bail!(
                "The specified path ({}) is not a directory",
                dense_path.display()
            );
        }

        // Run a fetch in the remote repository
        log::info!("Fetching in the dense repository");
        let dense_fetch_result = Command::new("git")
            .current_dir(&dense_path)
            .arg("fetch")
            .arg(opt.remote)
            .arg(&branch)
            .status()
            .context("running git-fetch in the dense repo")?;
        if !dense_fetch_result.success() {
            bail!("Fetching in the dense repo failed");
        }

        // Switch to the current ref, discarding work
        let dense_switch_result = Command::new("git")
            .current_dir(&dense_path)
            .arg("switch")
            .arg(&branch)
            .arg("--discard-changes")
            .status()
            .context("running git-switch")?;
        if !dense_switch_result.success() {
            bail!("Switching in the dense repo failed");
        }

        Ok(())
    }

    pub(crate) fn run_event_loop(&self) -> Result<()> {
        log::info!("Processing events");

        let buffered_stdin = BufReader::new(std::io::stdin());
        let mut line_number: usize = 0;
        for line in buffered_stdin.lines() {
            line_number += 1;
            let line = line.with_context(|| format!("reading stdin (line {})", line_number))?;
            log::info!("stdin:{}: '{}'", line_number, &line);
            self.handle_command(&line);
        }

        Ok(())
    }

    fn describe_capabilities(&self) -> Result<()> {
        println!("fetch");
        println!("");
        Ok(())
    }

    // TODO: Consider limiting refs we exhibit...
    fn remote_refs_to_objects(&self) -> Result<HashMap<String, String>> {
        let mut results = HashMap::<String, String>::new();

        let output = Command::new("git")
            .current_dir(&self.flags.remote_path()?)
            .arg("for-each-ref")
            .arg("--format=%(objectname) %(refname)")
            .arg("refs/heads/")
            .stderr(Stdio::inherit())
            .output()
            .context("running git for-each-ref")?;

        for line in output.stdout.lines() {
            let line = line.context("reading output of for-each-ref")?;
            let mut tokens = line.split_ascii_whitespace();

            let object_name = unwrap_or_bail(tokens.next()).context("expected object name token")?;
            let ref_name = unwrap_or_bail(tokens.next()).context("expected ref name token")?;

            results.insert(
                ref_name.to_owned(),
                object_name.to_owned(),
            );
        }

        Ok(results)
    }

    fn remote_symbolic_ref(&self, ref_name: &str) -> Result<String> {
        let output = Command::new("git")
            .current_dir(&self.flags.remote_path()?)
            .arg("symbolic-ref")
            .arg(&ref_name)
            .stderr(Stdio::inherit())
            .output()
            .context("running git symbolic-ref")?;

        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }

    fn list_refs(&self) -> Result<()> {
        let refs_to_objects = self
            .remote_refs_to_objects()
            .context("reading refs in the dense repo")?;
        let head_object = self
            .remote_symbolic_ref("HEAD")
            .context("resolving HEAD in the dense repo")?;

        for (ref_name, object_name) in refs_to_objects {
            log::info!("{} {}", &object_name, &ref_name);
            println!("{} {}", &object_name, &ref_name);
        }
        log::info!("@{} HEAD", &head_object);
        println!("@{} HEAD", &head_object);
        println!("");
        Ok(())
    }

    fn fetch(&self, oid: &str, ref_name: &str) -> Result<()> {
        
        todo!("implement me");
    }

    fn handle_command(&self, line: &String) -> Result<()> {
        let mut tokens = line.split_ascii_whitespace();
        if let Some(command) = tokens.next().take() {
            match command {
                "capabilities" => self.describe_capabilities()?,

                "list" => self.list_refs()?,

                "fetch" => {
                    let oid = unwrap_or_bail(tokens.next()).context("reading oid token")?;
                    let ref_name = unwrap_or_bail(tokens.next()).context("reading ref name token")?;
                    self.fetch(oid, ref_name)?;
                },

                "" => {
                    log::info!("Client terminated event stream normally");
                    return Ok(());
                }

                _ => {
                    bail!("unsupported command '{}'", command);
                }
            }

        } else {
            bail!("could not read command token");
        }

        Ok(())
    }
}

fn unwrap_or_bail<I>(option: Option<I>) -> Result<I> {
    if option.is_none() {
        bail!("expected something")
    }
    Ok(option.unwrap())
}

fn main() -> Result<()> {
    let flags = Flags::from_args();
    env_logger::Builder::from_env(Env::default().default_filter_or(&flags.default_log_level))
        .init();
    flags.validate().context("validating flags")?;

    let helper = Helper::new(flags)?;
    helper.run_event_loop()
}
