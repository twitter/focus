use anyhow::{bail, Context, Error, Result};
use std::borrow::Borrow;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufWriter;
use std::os::unix::fs::symlink;
use std::os::unix::prelude::OsStrExt;
use std::path::Path;
use std::thread;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Barrier},
};

use crate::model::{self, Layer, LayerSet, LayerSets, RichLayerSet};
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

pub fn configure_sparse_repo_initial(sparse_repo: &PathBuf, _sandbox: &Sandbox) -> Result<()> {
    // TODO: Consider enabling the fsmonitor after it can be bundled.
    // git_config(sparse_repo, "core.fsmonitor", "rs-git-fsmonitor", sandbox)?;

    Ok(())
}

fn set_up_alternate(sparse_repo: &Path, dense_repo: &Path) -> Result<()> {
    // use std::os::unix::ffi::OsStrExt;
    let alternates_path = sparse_repo.join(".git").join("info").join("alternates");
    let mut buf = Vec::from(dense_repo.as_os_str().as_bytes());
    buf.push(b'\n');
    std::fs::write(alternates_path, buf)?;

    Ok(())
}

fn configure_sparse_repo_final(
    dense_repo: &PathBuf,
    sparse_repo: &PathBuf,
    sandbox: &Sandbox,
) -> Result<()> {
    let dense_git_dir = dense_repo.join(".git");
    let sparse_git_dir = sparse_repo.join(".git");
    let dense_config = dense_git_dir.join("config");
    let sparse_config = sparse_git_dir.join("config");
    std::fs::copy(dense_config, sparse_config)
        .context("copying repo configuration the dense repo to the sparse repo")?;

    let paths_to_copy = vec!["config", "hooks", "hooks_multi", "repo.d"];
    for name in paths_to_copy {
        let from = dense_git_dir.join(name);
        if !from.exists() {
            log::warn!("Dense path {} does not exist!", &from.display());
            continue;
        }
        let to = sparse_git_dir.join(name);
        let (mut cmd, scmd) = SandboxCommand::new("cp", sandbox)?;
        scmd.ensure_success_or_log(
            cmd.arg("-v").arg("-r").arg(&from).arg(&to),
            SandboxCommandOutput::Stderr,
            &format!("Copying {} -> {}", &from.display(), &to.display()),
        )?;
    }

    // Add a URL to the dense repo
    let dense_url = format!("file://{}", dense_repo.to_str().unwrap());
    let (mut cmd, scmd) = git_command(sandbox)?;
    scmd.ensure_success_or_log(
        cmd.arg("remote")
            .arg("add")
            .arg("dense")
            .arg(&dense_url)
            .current_dir(&sparse_repo),
        SandboxCommandOutput::Stderr,
        &format!("adding dense remote ({})", &dense_url),
    )?;

    Ok(())
}

fn create_dense_link(dense_repo: &PathBuf, sparse_repo: &PathBuf) -> Result<()> {
    let link_path = sparse_repo.join(".dense");
    symlink(dense_repo, link_path).map_err(|e| Error::new(e))
}
// Write an object to a repo returning its identity.
fn git_hash_object(repo: &PathBuf, file: &PathBuf, sandbox: &Sandbox) -> Result<String> {
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

pub fn coordinates_from_layers(
    repo: &PathBuf,
    layer_names: Vec<&str>,
    sandbox: &Sandbox,
) -> Result<Vec<String>> {
    let layer_sets = LayerSets::new(&repo);
    let rich_layer_set = RichLayerSet::new(
        layer_sets
            .available_layers()
            .context("getting available layers")?,
    )?;
    let mut coordinates = HashSet::<String>::new();
    for layer_name in layer_names {
        if let Some(layer) = rich_layer_set.get(layer_name) {
            for coordinate in layer.coordinates() {
                coordinates.insert(coordinate.to_owned());
            }
        } else {
            bail!("Layer named '{}' not found", &layer_name)
        }
    }

    Ok(coordinates.into())
}

pub fn write_adhoc_layer_set(sparse_repo: &PathBuf, layer_set: &LayerSet) -> Result<()> {
    let layer_sets = LayerSets::new(sparse_repo);
    layer_sets.store_adhoc_layers(layer_set)
}

pub enum Spec {
    Coordinates(Vec<String>),
    Layers(Vec<String>),
}

pub fn create_sparse_clone(
    name: &String,
    dense_repo: &PathBuf,
    sparse_repo: &PathBuf,
    branch: &String,
    spec: &Spec,
    filter_sparse: bool,
    sandbox: Arc<Sandbox>,
) -> Result<()> {
    let layer_set = match spec {
        Spec::Coordinates(coordinates) => {
            // Put coordinates them into the "ad hoc" layer.
            LayerSet {
                layers: vec![Layer::new("adhoc", "Ad hoc layer", false, coordinates)],
                content_hash: None,
            }
        }

        Spec::Layers(layers) => {
            // TODO: Refactor this. We need a plural retrieval function on LayerSet or something.
            let layer_sets = model::LayerSets::new(&dense_repo);
            let available_layers = RichLayerSet::new(layer_sets.available_layers()?)?;
            let mut missing: bool;
            let found_layers = Vec::<Layer>::new();

            for layer_name in layers {
                if let Some(layer) = available_layers.get(name) {
                    found_layers.push(layer.to_owned());
                } else {
                    bail!("Layer {} not found", layer_name);
                }
            }

            LayerSet {
                layers: found_layers,
                content_hash: None,
            }
        }
    };

    let coordinates = Vec::<String>::new();
    for layer in layer_set.layers() {
        coordinates.extend(layer.coordinates().borrow());
    }

    create_or_update_sparse_clone(
        &name,
        &dense_repo,
        &sparse_repo,
        &branch,
        &coordinates,
        filter_sparse,
        true,
        sandbox,
    )?;

    if !adhoc_layer_set.layers().is_empty() {
        write_adhoc_layer_set(&sparse_repo, adhoc_layer_set)?;
    }

    Ok(())
}

pub fn create_or_update_sparse_clone(
    name: &String,
    dense_repo: &PathBuf,
    sparse_repo: &PathBuf,
    branch: &String,
    coordinates: &Vec<String>,
    filter_sparse: bool,
    create: bool,
    sandbox: Arc<Sandbox>,
) -> Result<()> {
    // TODO: Crash harder in threads to prevent extra work.

    if create {
        if sparse_repo.is_dir() {
            bail!("Sparse repo already exists and creation was requested")
        }
    } else if !sparse_repo.is_dir() {
        bail!("Sparse repo does not exist and creation is not allowed")
    }

    let sparse_profile_output = sandbox.path().join("sparse-checkout");
    let project_view_output = {
        let project_view_name = format!("focus-{}.bazelproject", &name);
        sandbox.path().join(project_view_name)
    };

    configure_dense_repo(&dense_repo, sandbox.as_ref())
        .context("setting configuration options in the dense repo")?;

    // Being on the right branch in the dense repository is a prerequisite for any work.
    switch_to_detached_branch_discarding_changes(&dense_repo, &branch, sandbox.as_ref())?;

    let profile_generation_barrier = Arc::new(Barrier::new(2));
    let profile_generation_thread = {
        let cloned_sandbox = sandbox.clone();
        let cloned_dense_repo = dense_repo.to_owned();
        let cloned_sparse_profile_output = sparse_profile_output.to_owned();
        let cloned_coordinates = coordinates.clone();
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
        let cloned_coordinates = coordinates.clone();

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
        if !create {
            return Ok(());
        }

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
                configure_sparse_repo_initial(&cloned_sparse_repo, &cloned_sandbox)
                    .expect("failed to configure sparse clone");
                log::info!("Finished creating a template clone");
            })
    };
    clone_thread?
        .join()
        .expect("clone thread exited abnormally");

    // N.B. We set up an alternate because it allows for journaled fetches
    set_up_alternate(&sparse_repo, &dense_repo).context("setting up an alternate")?;

    if !filter_sparse {
        // If we haven't awaited the profile generation thread, we we must now.
        profile_generation_barrier.wait();
    }

    profile_generation_joinable
        .join()
        .expect("sparse profile generation thread exited abnormally");
    {
        let cloned_sandbox = sandbox.clone();
        let cloned_sparse_repo = sparse_repo.clone();
        let cloned_dense_repo = dense_repo.clone();

        log::info!("Configuring visible paths");
        set_sparse_checkout(sparse_repo, &sparse_profile_output, &cloned_sandbox)
            .context("setting up sparse checkout options")?;

        log::info!("Finalizing configuration");
        if create {
            configure_sparse_repo_final(&cloned_dense_repo, &cloned_sparse_repo, &cloned_sandbox)
                .context("failed to perform final configuration in the sparse repo")?;
            create_dense_link(&cloned_dense_repo, &cloned_sparse_repo)
                .context("failed to create a link to the dense repo in the sparse repo")?;
        }

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
        let sparse_profile_destination = sparse_repo
            .join(".git")
            .join("info")
            .join("sparse-checkout");
        // std::fs::copy(&sparse_profile, &sparse_profile_destination).context("copying the sparse-checkout file into place")?;
        // TODO: If the git version supports it, add --no-sparse-index since the sparse index performs poorly
        log::info!("Setting sparse from {}", &sparse_profile.display());
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
                .arg("add")
                .arg("--stdin"),
            SandboxCommandOutput::Stderr,
            "sparse-checkout add",
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
            .arg("1")
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
    let info_dir = &sparse_repo.join(".git").join("info");
    let excludes_path = &info_dir.join("excludes");
    {
        use std::fs::OpenOptions;
        let mut buffer = BufWriter::new(
            OpenOptions::new()
                .write(true)
                .create(true)
                .append(true)
                .open(excludes_path)
                .context("opening the info/excludes file for writing")?,
        );

        writeln!(buffer, "WORKSPACE.focus")?;
        writeln!(buffer, "BUILD.focus")?;
        writeln!(buffer, "*_focus.bzl")?;
        writeln!(buffer, "focus-*.bazelproject")?;
        writeln!(buffer, "focus-*.bazelrc")?;
        writeln!(buffer, "")?;
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

    let f = File::create(&bazel_project_view_path).context("creating output file")?;

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
            buffer.write_all(&path_bytestring[..])?;
            writeln!(buffer, "")?;
        }
    }
    writeln!(buffer, "")?;
    buffer.flush()?;

    Ok(())
}

pub fn switch_to_detached_branch_discarding_changes(
    repo: &Path,
    refname: &str,
    sandbox: &Sandbox,
) -> Result<()> {
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

fn generate_sparse_profile(
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
    pub fn new(dense_repo: &Path, _coordinates: Vec<String>) -> Result<Self> {
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
