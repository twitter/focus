use anyhow::{bail, Context, Result};
use env_logger::{self, Env};
use futures::io::empty;
use std::{
    cell::Cell,
    collections::HashMap,
    convert::TryInto,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{exit, Command, Stdio},
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
    branch: String,
}

impl Helper {
    pub(crate) fn new(flags: Flags) -> Result<Self> {
        let branch =
            Self::current_branch(&flags.sparse_repo).context("determining the current branch")?;
        Ok(Self { flags, branch })
    }

    pub(crate) fn current_branch(dir: &Path) -> Result<String> {
        let output = Command::new("git")
            .arg("branch")
            .arg("--show-current")
            .current_dir(dir)
            .output()
            .context("running git-branch")?;

        Ok(String::from_utf8(output.stdout)
            .context("interpreting output as UTF-8")?
            .trim()
            .to_owned())
    }

    pub(crate) fn fetch_from_upstream(&self) -> Result<()> {
        // Run a fetch in the remote repository
        let dense_path = self.flags.remote_path()?;
        log::info!("Fetching in the dense repository");
        let dense_fetch_result = Command::new("git")
            .current_dir(&dense_path)
            .arg("fetch")
            .arg(&self.flags.remote)
            .arg(&self.branch)
            .stdout(Stdio::null())
            .status()
            .context("running git-fetch in the dense repo")?;
        if !dense_fetch_result.success() {
            bail!("Fetching in the dense repo failed");
        }

        // // Switch to the current ref, discarding work
        // let dense_switch_result = Command::new("git")
        //     .current_dir(&dense_path)
        //     .arg("switch")
        //     .arg(&branch)
        //     .arg("--discard-changes")
        //     .status()
        //     .context("running git-switch")?;
        // if !dense_switch_result.success() {
        //     bail!("Switching in the dense repo failed");
        // }

        Ok(())
    }

    pub(crate) fn fetch_from_dense(&self) -> Result<()> {
        let dense_path = self.flags.remote_path()?;
        // let mut prefixed_branch = String::from("refs/remotes/origin/");
        // prefixed_branch.push_str(&self.branch);
        log::info!(
            "Fetching in the sparse repository",
            // prefixed_branch
        );
        // Redirect stdout
        let sparse_fetch_result = Command::new("git")
            .arg("fetch")
            .arg("--no-tags")
            .arg("dense")
            .arg(&self.branch)
            .stdout(Stdio::null())
            .status()
            .context("running git-fetch in the sparse repo")?;

        if !sparse_fetch_result.success() {
            bail!("Fetching in the sparse repo failed");
        }

        Ok(())
    }

    pub(crate) fn run_upload_pack(&self) -> Result<()> {
        let remote_path = self.flags.remote_path()?;
        log::info!("Fetching from the dense repository");
        let status = Command::new("git")
            .arg("upload-pack")
            .arg(remote_path)
            // .env("GIT_PROTOCOL", "version=1")
            // .env("GIT_PREFIX", "")
            .stdin(Stdio::inherit())
            .status()
            .context("running git-upload-pack")?;

        if status.success() {
            bail!("upload-pack failed");
        }

        Ok(())
    }

    pub(crate) fn run_event_loop(&self) -> Result<()> {
        let buffered_stdin = BufReader::new(std::io::stdin());
        let mut lines = buffered_stdin.lines();

        loop {
            if let Some(line) = lines.next() {
                let line = line.context("reading stdin")?;
                log::info!("[primary] {}", line);
                if line.is_empty() {
                    log::info!("Client signalled end of event stream");
                    break;
                }

                let mut tokens = line.split_ascii_whitespace();
                let verb = unwrap_or_bail(tokens.next()).context("reading verb")?;
                match verb {
                    "capabilities" => self.describe_capabilities()?,

                    "list" => self.list_refs()?,

                    "option" => {
                        let key = unwrap_or_bail(tokens.next()).context("reading option key")?;
                        let value = unwrap_or_bail(tokens.next()).context("reading option value")?;
                        log::info!("option: {}={}", &key, &value);
                        println!("unsupported"); // Whatever ;-)
                    }

                    "fetch" => {
                        // Accumulate fetch requests
                        let mut wanted_oids = Vec::<String>::new();
                        let oid = unwrap_or_bail(tokens.next()).context("reading oid")?;
                        let _ref_name =
                            unwrap_or_bail(tokens.next()).context("reading ref name")?;
                        wanted_oids.push(oid.to_owned());

                        let mut accumulating = true;
                        while accumulating {
                            let line = lines.next().unwrap();
                            let line = line.context("reading fetch request lines")?;
                            log::info!("[fetch request] '{}'", line);
                            if !line.is_empty() {
                                let mut tokens = line.split_ascii_whitespace();
                                let cmd =
                                    unwrap_or_bail(tokens.next()).context("reading command")?;
                                if !cmd.eq_ignore_ascii_case("fetch") {
                                    bail!("unexpected command {}", &cmd);
                                }
                                let oid = unwrap_or_bail(tokens.next()).context("reading oid")?;
                                let _ref_name =
                                    unwrap_or_bail(tokens.next()).context("reading ref name")?;
                                wanted_oids.push(oid.to_owned());
                            } else {
                                log::info!("Got last fetch request");
                                accumulating = false;
                            }
                        }

                        // Fetch the requested objects
                        // TODO: Fix this jank.
                        for oid in wanted_oids {
                            self.fetch_pack_from_dense(&oid)
                                .with_context(|| format!("fetching a pack for {}", &oid))?;
                        }
                        exit(0);
                    }

                    "" => {
                        log::info!("end of stream");
                        exit(0);
                    }

                    _ => {
                        bail!("unsupported command '{}'", verb);
                    }
                }
            }
        }

        Ok(())
    }

    fn describe_capabilities(&self) -> Result<()> {
        // println!("option");
        println!("fetch");
        println!("");
        Ok(())
    }

    fn remote_refs_to_objects(&self) -> Result<HashMap<String, String>> {
        let mut results = HashMap::<String, String>::new();

        let output = Command::new("git")
            .current_dir(&self.flags.remote_path()?)
            .arg("for-each-ref")
            .arg("--format=%(objectname) %(refname)")
            .arg(format!("refs/heads/{}", &self.branch))
            .stderr(Stdio::inherit())
            .output()
            .context("running git for-each-ref")?;

        for line in output.stdout.lines() {
            let line = line.context("reading output of for-each-ref")?;
            let mut tokens = line.split_ascii_whitespace();

            let object_name =
                unwrap_or_bail(tokens.next()).context("expected object name token")?;
            let ref_name = unwrap_or_bail(tokens.next()).context("expected ref name token")?;

            results.insert(ref_name.to_owned(), object_name.to_owned());
        }

        Ok(results)
    }

    fn remote_symbolic_ref(&self, ref_name: &str) -> Result<String> {
        // let output = Command::new("git")
        //     .current_dir(&self.flags.remote_path()?)
        //     .arg("rev-parse")
        //     .arg(&ref_name)
        //     .stderr(Stdio::inherit())
        //     .output()
        //     .context("running git symbolic-ref")?;

        // Ok(String::from_utf8(output.stdout)?.trim().to_owned())
        Ok(format!("refs/heads/{}", &self.branch))
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

    fn fetch_pack_from_dense(&self, oid: &str) -> Result<()> {
        log::info!("Running fetch {}", oid);
        let status = Command::new("git")
            .current_dir(&self.flags.sparse_repo)
            .stderr(Stdio::inherit())
            .stdout(Stdio::null())
            .arg("fetch-pack")
            .arg("-v")
            .arg("--check-self-contained-and-connected")
            .arg(self.flags.url.to_string())
            .arg(oid)
            .status()
            .context("running git fetch-pack")?;
        if !status.success() {
            bail!("Fetching from the dense repo failed");
        }

        Ok(())
    }
}

pub fn unwrap_or_bail<I>(option: Option<I>) -> Result<I> {
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

    helper.fetch_from_upstream().context("fetching from upstream into dense")?;

    // helper.fetch_from_dense().context("fetching from dense into sparse")?;
    
    // let refname = format!("refs/heads/{}", &helper.branch);
    // helper.fetch_pack_from_dense(&refname).context("fetching pack from dense")?;

    helper.run_event_loop().context("running the helper")?;

    Ok(())
}
