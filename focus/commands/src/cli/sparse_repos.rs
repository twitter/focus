use anyhow::{bail, Context, Result};
use git2::Repository;
use std::ffi::OsString;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufWriter;
use std::path::Path;
use std::thread;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Barrier},
};

use crate::sandbox::Sandbox;
use crate::sandbox_command::SandboxCommand;
use crate::sandbox_command::SandboxCommandOutput;

const SPARSE_PROFILE_PRELUDE: &str = "/tools/\n/pants-plugins/\n/pants-support/\n/3rdparty/\n";

fn add_implicit_coordinates(v: &mut Vec<String>) {
    let implicit_coordinates: Vec<String> = vec![
        String::from("//tools/implicit_deps:thrift-implicit-deps-impl"),
        String::from("//scrooge-internal/..."),
        String::from("//loglens/loglens-logging/..."),
    ];
    v.extend(implicit_coordinates)
}

fn git_binary() -> OsString {
    OsString::from("git")
}

fn git_command(sandbox: &Sandbox) -> Result<(Command, SandboxCommand)> {
    SandboxCommand::new(git_binary(), sandbox)
}

fn git_config<P: AsRef<Path>>(repo_path: P, key: &str, val: &str, sandbox: &Sandbox) -> Result<()> {
    let (mut cmd, scmd) = git_command(&sandbox)?;
    scmd.ensure_success_or_log(
        cmd.current_dir(repo_path).arg("config").arg(key).arg(val),
        SandboxCommandOutput::Stderr,
        "git config",
    )
    .map(|_| ())
}

pub fn configure_dense_repo(dense_repo: &PathBuf, sandbox: &Sandbox) -> Result<()> {
    git_config(dense_repo, "uploadPack.allowFilter", "true", sandbox)
}

pub fn configure_sparse_repo(sandbox: &Sandbox, sparse_repo: &PathBuf) -> Result<()> {
    // TODO: Consider enabling the fsmonitor after it can be bundled.
    // git_config(sparse_repo, "core.fsmonitor", "rs-git-fsmonitor", sandbox)?;
    Ok(())
}

// Write an object to a repo returning its identity.
pub fn git_hash_object(repo: &PathBuf, file: &PathBuf, sandbox: &Sandbox) -> Result<String> {
    let (mut cmd, scmd) = git_command(&sandbox)?;
    if let Err(e) = cmd
        .current_dir(repo)
        .arg("hash-object")
        .arg("-w")
        .arg(file)
        .status()
    {
        scmd.log(
            crate::sandbox_command::SandboxCommandOutput::Stderr,
            &"failed 'git hash-object' command",
        )?;
        bail!("git hash-object failed: {}", e);
    }
    let mut stdout_contents = String::new();
    scmd.read_to_string(SandboxCommandOutput::Stdout, &mut stdout_contents)?;
    Ok(stdout_contents.trim().to_owned())
}

pub fn create_sparse_clone(
    name: &String,
    dense_repo: &PathBuf,
    sparse_repo: &PathBuf,
    branch: &String,
    coordinates: &Vec<String>,
    filter_sparse: bool,
    sandbox: Arc<Sandbox>,
) -> Result<()> {
    // TODO: Crash harder in threads to prevent extra work. 

    let sparse_profile_output = sandbox.path().join("sparse-checkout");
    let project_view_output = {
        let project_view_name = format!("focus-{}.bazelproject", &name);
        sandbox.path().join(project_view_name)
    };

    let computed_coordinates = {
        let mut coordinates = coordinates.clone();
        add_implicit_coordinates(&mut coordinates);
        coordinates
    };

    configure_dense_repo(&dense_repo, sandbox.as_ref())
        .context("setting configuration options in the dense repo")?;
    
    // Being on the right branch in the dense repsitory is a prerequisite for any work.
    switch_to_detached_branch_discarding_changes(&dense_repo, &branch, sandbox.as_ref())?;

    let profile_generation_barrier = Arc::new(Barrier::new(2));
    let profile_generation_thread = {
        let cloned_sandbox = sandbox.clone();
        let cloned_dense_repo = dense_repo.to_owned();
        let cloned_sparse_profile_output = sparse_profile_output.to_owned();
        let cloned_coordinates = computed_coordinates.to_vec().clone();
        let cloned_profile_generation_barrier = profile_generation_barrier.clone();

        thread::Builder::new()
            .name("SparseProfileGeneration".to_owned())
            .spawn(move || {
                log::info!("Generating sparse profile");
                generate_sparse_profile(
                    &cloned_dense_repo,
                    &cloned_sparse_profile_output,
                    &cloned_coordinates,
                    &cloned_sandbox.as_ref(),
                )
                .expect("failed to generate a sparse profile");
                log::info!("Finished generating sparse profile");
                cloned_profile_generation_barrier.wait();
            })
    };

    let project_view_generation_thread = {
        let cloned_sandbox = sandbox.clone();
        let cloned_dense_repo = dense_repo.to_owned();
        let cloned_project_view_output = project_view_output.clone();
        let cloned_coordinates = computed_coordinates.to_vec().clone();

        thread::Builder::new()
            .name("ProjectViewGeneration".to_owned())
            .spawn(move || {
                log::info!("Generating project view");
                write_project_view_file(
                    &cloned_dense_repo,
                    &cloned_project_view_output,
                    &cloned_coordinates,
                    &cloned_sandbox.as_ref(),
                )
                .expect("generating the project view");
                log::info!("Finished generating project view");
            })
    };

    project_view_generation_thread?
        .join()
        .expect("project view generation thread exited abnormally");

    let profile_generation_joinable = profile_generation_thread.context("getting joinable")?;
    if filter_sparse {
        // If we filter using the 'sparse' technique, we have to wait for the sparse profile to be
        // complete before cloning (since it reads it from the dense repo during clone).
        profile_generation_barrier.wait();
    }

    let clone_thread = {
        let cloned_sandbox = sandbox.clone();
        let cloned_dense_repo = dense_repo.to_owned();
        let cloned_sparse_repo = sparse_repo.to_owned();
        let cloned_branch = branch.clone();
        let cloned_sparse_profile_output = sparse_profile_output.clone();

        thread::Builder::new()
            .name("CloneRepository".to_owned())
            .spawn(move || {
                log::info!("Creating a template clone");
                create_empty_sparse_clone(
                    &cloned_dense_repo,
                    &cloned_sparse_repo,
                    &cloned_branch,
                    &cloned_sparse_profile_output,
                    filter_sparse,
                    &cloned_sandbox,
                )
                .expect("failed to create an empty sparse clone");
                configure_sparse_repo(&cloned_sandbox, &cloned_sparse_repo)
                    .expect("failed to configure sparse clone");
                log::info!("Finished creating a template clone");
            })
    };
    clone_thread?
        .join()
        .expect("clone thread exited abnormally");

    if !filter_sparse {
        // If we haven't awaited the profile generation thread, we we must now.
        profile_generation_barrier.wait();
    }

    profile_generation_joinable
        .join()
        .expect("sparse profile generation thread exited abnormally");
    {
        let cloned_sandbox = sandbox.clone();
        let cloned_sparse_repo = sparse_repo.to_owned();

        log::info!("Configuring visible paths");
        set_sparse_checkout(sparse_repo, &sparse_profile_output, &cloned_sandbox)
            .context("setting up sparse checkout options")?;

        log::info!("Checking out the working copy");
        checkout_working_copy(&cloned_sparse_repo, &cloned_sandbox)
            .context("switching branches")?;

        log::info!("Moving the project view into place");
        let project_view_file_name = &project_view_output
            .file_name()
            .context("getting the file name failed")?;
        let project_view_destination = &cloned_sparse_repo.join(&project_view_file_name);
        std::fs::rename(project_view_output, project_view_destination)
            .context("copying in the project view")?;
    }
    Ok(())
}

pub fn set_sparse_checkout(
    sparse_repo: &PathBuf,
    sparse_profile: &PathBuf,
    sandbox: &Sandbox,
) -> Result<()> {
    {
        // TODO: If the git version supports it, add --no-sparse-index since the sparse index performs poorly
        let (mut cmd, scmd) = git_command(&sandbox)?;
        scmd.ensure_success_or_log(
            cmd.current_dir(sparse_repo)
                .arg("sparse-checkout")
                .arg("init")
                .arg("--cone"),
            SandboxCommandOutput::Stderr,
            "sparse-checkout init",
        )
        .map(|_| ())
        .context("initializing sparse checkout")?;
    }

    {
        // TODO: If the git version supports it, add --no-sparse-index since the sparse index performs poorly

        // Start a sparse-checkout with the sparse profile file as input.
        let sparse_profile_file = File::open(&sparse_profile).context("opening sparse profile")?;
        let (mut cmd, scmd) = SandboxCommand::new_with_handles(
            git_binary(),
            Some(Stdio::from(sparse_profile_file)),
            None,
            None,
            &sandbox,
        )?;
        scmd.ensure_success_or_log(
            cmd.current_dir(sparse_repo)
                .arg("sparse-checkout")
                .arg("set"),
            SandboxCommandOutput::Stderr,
            "sparse-checkout set",
        )
        .map(|_| ())
        .context("initializing sparse checkout")?;
    }

    Ok(())
}

pub fn checkout_working_copy(sparse_repo: &PathBuf, sandbox: &Sandbox) -> Result<()> {
    // TODO: If the git version supports it, add --no-sparse-index since the sparse index performs poorly
    let (mut cmd, scmd) = git_command(&sandbox)?;
    scmd.ensure_success_or_log(
        cmd.current_dir(sparse_repo).arg("checkout"),
        SandboxCommandOutput::Stderr,
        "checkout",
    )
    .map(|_| ())
    .context("checking out the working copy")
}

pub fn create_empty_sparse_clone(
    dense_repo: &PathBuf,
    sparse_repo: &PathBuf,
    branch: &String,
    sparse_profile_output: &PathBuf,
    filter_sparse: bool,
    sandbox: &Sandbox,
) -> Result<()> {
    let mut dense_url = OsString::from("file://");
    dense_url.push(dense_repo);

    let sparse_repo_dir_parent = &sparse_repo
        .parent()
        .context("sparse repo parent directory does not exist")?;

    let filter_arg = if filter_sparse {
        let profile_object_id = git_hash_object(&dense_repo, &sparse_profile_output, sandbox)
            .context("writing sparse profile into dense repo")?;
        log::info!(
            "Wrote the sparse profile into the dense repo as {}",
            &profile_object_id
        );
        format!("--filter=sparse:oid={}", &profile_object_id).to_owned()
    } else {
        "--filter=blob:none".to_owned()
    };

    log::info!(
        "Dense repo: {}, sparse repo: {}",
        &dense_repo.display(),
        &sparse_repo.display()
    );
    // TODO: If the git version supports it, add --no-sparse-index since the sparse index performs poorly
    let (mut cmd, scmd) = git_command(&sandbox)?;
    scmd.ensure_success_or_log(
        cmd.current_dir(sparse_repo_dir_parent)
            .arg("clone")
            .arg("--sparse")
            .arg("--no-checkout")
            .arg("--no-tags")
            .arg("--single-branch")
            .arg("--depth")
            .arg("64")
            .arg("-b")
            .arg(branch.as_str())
            .arg(filter_arg)
            .arg(dense_url)
            .arg(sparse_repo),
        SandboxCommandOutput::Stderr,
        "clone",
    )
    .map(|_| ())
    .context("creating the sparse clone")?;

    // Write an excludes file that ignores Focus-specific modifications in the sparse repo.
    let info_dir = &dense_repo.join(".git").join("info");
    std::fs::create_dir_all(info_dir);
    let excludes_path = &info_dir.join("excludes.focus");
    {
        use std::fs::OpenOptions;
        let mut buffer = BufWriter::new(
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(excludes_path)
                .context("opening the info/excludes file for writing")?,
        );
        buffer.write_all(b"WORKSPACE.focus\n")?;
        buffer.write_all(b"BUILD.focus\n")?;
        buffer.write_all(b"*_focus.bzl\n")?;
        buffer.write_all(b"focus-*.bazelproject\n")?;
        buffer.write_all(b"focus-*.bazelrc\n")?;
        buffer.write_all(b"\n")?;
        buffer.flush()?;
    }

    Ok(())
}

fn allowable_project_view_directory_predicate(dense_repo: &Path, directory: &Path) -> bool {
    let scrooge_internal = dense_repo.join("scrooge-internal");
    let loglens = dense_repo.join("loglens");
    !directory.starts_with(scrooge_internal) && !directory.starts_with(loglens)
}

fn write_project_view_file(
    dense_repo: &PathBuf,
    bazel_project_view_path: &Path,
    coordinates: &Vec<String>,
    sandbox: &Sandbox,
) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let client = BazelRepo::new(dense_repo, coordinates.clone())?;
    let mut directories = BTreeSet::<PathBuf>::new();

    let directories_for_coordinate = client
        .involved_directories_query(&coordinates, Some(1), true, sandbox)
        .context("determining directories for project view")?;
    for dir in directories_for_coordinate {
        let dir_path = PathBuf::from(dir);
        if let Some(dir_with_build) = client
            .find_closest_directory_with_build_file(dir_path.as_path(), &dense_repo)
            .context("finding closest directory with a build file")?
        {
            if allowable_project_view_directory_predicate(&dense_repo.as_path(), &dir_with_build) {
                directories.insert(dir_with_build);
            }
        } else {
            log::warn!(
                "Ignoring directory '{}' as it has no discernible BUILD file",
                &dir_path.display()
            );
        }
    }

    if directories.is_empty() {
        bail!("Refusing to generate a project view with an empty set of directories.");
    }

    let mut f = File::create(&bazel_project_view_path).context("creating output file")?;

    let mut buffer = BufWriter::new(f);
    writeln!(buffer, "workspace_type: java")?;
    writeln!(buffer, "")?;
    writeln!(buffer, "additional_languages:")?;
    writeln!(buffer, "  scala")?;
    writeln!(buffer, "")?;
    writeln!(buffer, "derive_targets_from_directories: true")?;
    writeln!(buffer, "")?;
    writeln!(buffer, "directories:")?;
    let prefix = dense_repo
        .to_str()
        .context("interpreting prefix as utf-8")?;
    // TODO: Sort and dedup. Fix weird breaks
    for dir in &directories {
        let relative_path = dir.strip_prefix(&prefix);
        let path_bytestring = relative_path
            .context("truncating path")?
            .as_os_str()
            .as_bytes();
        if !path_bytestring.is_empty() {
            write!(buffer, "  ")?;
            buffer.write(&path_bytestring[..])?;
            writeln!(buffer, "")?;
        }
    }
    writeln!(buffer, "")?;
    buffer.flush()?;

    Ok(())
}

pub fn switch_to_detached_branch_discarding_changes(repo: &Path, refname: &str, sandbox: &Sandbox) -> Result<()> {
    let (mut cmd, scmd) = git_command(sandbox)?;
    scmd.ensure_success_or_log(
        cmd.arg("switch")
            .arg(refname)
            .arg("--quiet")
            .arg("--detach")
            .arg("--discard-changes")
            .current_dir(&repo),
        SandboxCommandOutput::Stderr,
        &format!("switching to ref '{}' in repo {}", refname, &repo.display()),
    )?;

    Ok(())
}

pub fn generate_sparse_profile(
    dense_repo: &Path,
    sparse_profile_output: &Path,
    coordinates: &Vec<String>,
    sandbox: &Sandbox,
) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let client = BazelRepo::new(dense_repo, coordinates.clone())?;

    let repo_component_count = dense_repo.components().count();

    let mut query_dirs = BTreeSet::<PathBuf>::new();

    let directories_for_coordinate = client
        .involved_directories_query(&coordinates, None, false, sandbox)
        .with_context(|| format!("Determining involved directories for {:?}", &coordinates))?;
    log::info!(
        "Dependency query: {:?} yielded {} directories",
        &coordinates,
        &directories_for_coordinate.len()
    );
    for dir in directories_for_coordinate {
        let absolute_path = dense_repo.join(dir);
        query_dirs.insert(absolute_path);
    }

    let mut f = File::create(&sparse_profile_output).context("creating output file")?;
    f.write_all(&SPARSE_PROFILE_PRELUDE.as_bytes())
        .context("writing sparse profile prelude")?;
    for dir in &query_dirs {
        let mut line = Vec::<u8>::new();
        line.extend(b"/"); // Paths have a '/' prefix
        {
            let mut relative_path = PathBuf::new();
            // aquery returns explicit paths. relativize them.
            for component in dir.components().skip(repo_component_count) {
                relative_path.push(component);
            }

            line.extend(relative_path.as_os_str().as_bytes());
        }
        line.extend(b"/\n"); // Paths have a '/' suffix
        f.write_all(&line[..])
            .with_context(|| format!("writing output (item={:?})", dir))?;
    }
    f.sync_data().context("syncing data")?;

    Ok(())
}

struct BazelRepo {
    dense_repo: PathBuf,
}

impl BazelRepo {
    pub fn new(dense_repo: &Path, coordinates: Vec<String>) -> Result<Self> {
        Ok(Self {
            dense_repo: dense_repo.to_owned(),
        })
    }

    fn find_closest_directory_with_build_file(
        &self,
        file: &Path,
        ceiling: &Path,
    ) -> Result<Option<PathBuf>> {
        let mut dir = if file.is_dir() {
            file
        } else {
            file.parent().context("getting parent directory of file")?
        };
        loop {
            if dir == ceiling {
                return Ok(None);
            }

            for entry in std::fs::read_dir(&dir)
                .with_context(|| format!("reading directory contents {}", dir.display()))?
            {
                let entry = entry.context("reading directory entry")?;
                if entry.file_name() == "BUILD" {
                    // Match BUILD, BUILD.*
                    return Ok(Some(dir.to_owned()));
                }
            }

            dir = dir
                .parent()
                .context("getting parent of current directory")?;
        }
    }

    // Use bazel query to get involved packages and turn them into directories.
    pub fn involved_directories_query(
        &self,
        coordinates: &Vec<String>,
        depth: Option<usize>,
        identity: bool,
        sandbox: &Sandbox,
    ) -> Result<Vec<String>> {
        // N.B. `bazel aquery` cannot handle unions ;(
        let mut directories = Vec::<String>::new();

        let clauses: Vec<String> = coordinates
            .iter()
            .map(|coordinate| {
                if identity {
                    coordinate.to_owned()
                } else if let Some(depth) = depth {
                    // format!("{}deps({}, {})", identity_clause, coordinate, depth)
                    format!("buildfiles(deps({}, {}))", coordinate, depth)
                } else {
                    format!("buildfiles(deps({}))", coordinate)
                }
            })
            .collect();

        let query = clauses.join(" union ");
        log::info!("Running Bazel query [{}]", &query);

        // Run Bazel query
        let (mut cmd, scmd) = SandboxCommand::new("./bazel", sandbox)?;
        scmd.ensure_success_or_log(
            cmd.arg("query")
                .arg(query)
                .arg("--output=package")
                .current_dir(&self.dense_repo),
            SandboxCommandOutput::Stderr,
            "bazel query",
        )?;

        let reader = scmd.read_buffered(SandboxCommandOutput::Stdout)?;
        for line in reader.lines() {
            if let Ok(line) = line {
                let path = PathBuf::from(&line);
                if !&line.starts_with("@")
                    && !path.starts_with("bazel-out/")
                    && !path.starts_with("external/")
                {
                    let absolute_path = &self.dense_repo.join(&path);
                    if let Some(path) = absolute_path.to_str() {
                        directories.push(path.to_owned());
                    } else {
                        bail!(
                            "Path '{}' contains characters that cannot be safely converted",
                            &path.display()
                        );
                    }
                }
            }
        }

        Ok(directories)
    }
}

fn reduce_to_shortest_common_prefix(paths: &BTreeSet<PathBuf>) -> Result<BTreeSet<PathBuf>> {
    let mut results = BTreeSet::<PathBuf>::new();
    let mut last_path: Option<PathBuf> = None;
    for path in paths {
        let insert = match &last_path {
            Some(last_path) => !path.starts_with(last_path),
            None => true,
        };

        if insert {
            results.insert(path.clone());
            last_path = Some(path.to_owned());
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reduce_to_shortest_common_prefix() -> Result<()> {
        let tempdir = tempfile::tempdir()?;
        let dir = tempdir.path();
        let mut set = BTreeSet::<PathBuf>::new();
        let a0 = dir.join("a0");
        let a0_b = a0.join("b");
        let a0_b_c = a0_b.join("c");
        let a1 = dir.join("a1");

        set.insert(a0_b.clone());
        set.insert(a0_b_c.clone());
        set.insert(a1.clone());

        let resulting_set = reduce_to_shortest_common_prefix(&set)?;

        assert_eq!(resulting_set.len(), 2);
        assert!(resulting_set.contains(&a0_b));
        assert!(resulting_set.contains(&a1));

        Ok(())
    }
}
