use anyhow::{bail, Context, Error, Result};
use internals::util::lock_file::LockFile;

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
    process::Stdio,
    sync::{Arc, Barrier},
};

use crate::git_helper::{self, git_binary, git_command};
use crate::model::{self, Layer, LayerSet, LayerSets, RichLayerSet};
use crate::sandbox::Sandbox;
use crate::sandbox_command::SandboxCommand;
use crate::sandbox_command::SandboxCommandOutput;
use crate::tracker::Tracker;
use crate::working_tree_synchronizer::WorkingTreeSynchronizer;

// TODO: Revisit this...
const SPARSE_PROFILE_PRELUDE: &str =
    "/tools/\n/pants-plugins/\n/pants-support/\n/3rdparty/\n/focus/\n";

pub fn configure_dense_repo(dense_repo: &PathBuf, sandbox: &Sandbox) -> Result<()> {
    git_helper::write_config(dense_repo, "uploadPack.allowFilter", "true", sandbox)
}

pub fn configure_sparse_repo_initial(_sparse_repo: &PathBuf, _sandbox: &Sandbox) -> Result<()> {
    Ok(())
}

fn set_up_alternates(sparse_repo: &Path, dense_repo: &Path) -> Result<()> {
    let alternates_path = sparse_repo.join(".git").join("info").join("alternates");
    let dense_pruned_odb = dense_repo.join(".git").join("pruned-odb").join("objects");
    let sparse_pruned_odb = sparse_repo.join(".git").join("pruned-odb").join("objects");

    std::fs::create_dir_all(&sparse_pruned_odb).context("creating sparse pruned-odb")?;

    let mut buf = Vec::from(dense_repo.as_os_str().as_bytes());
    buf.push(b'\n');
    buf.extend(dense_pruned_odb.as_os_str().as_bytes());
    buf.push(b'\n');
    buf.extend(sparse_pruned_odb.as_os_str().as_bytes());
    buf.push(b'\n');
    std::fs::write(alternates_path, buf)?;

    Ok(())
}

// Set git config key twitter.focus.sync-point to HEAD
fn configure_sparse_sync_point(sparse_repo: &PathBuf, sandbox: &Sandbox) -> Result<()> {
    let sync = WorkingTreeSynchronizer::new(sparse_repo.as_path(), sandbox)?;
    let head_str = String::from_utf8(sync.read_head()?)?;
    git_helper::write_config(
        sparse_repo,
        "twitter.focus.sync-point",
        head_str.as_str(),
        sandbox,
    )
}

// Set git config key twitter.focus.sync-point to HEAD
fn setup_bazel_preflight_script(sparse_repo: &PathBuf, _sandbox: &Sandbox) -> Result<()> {
    let sparse_focus_dir = sparse_repo.join(".focus");
    if !sparse_focus_dir.is_dir() {
        std::fs::create_dir(sparse_focus_dir.as_path()).with_context(|| {
            format!("failed to create directory {}", sparse_focus_dir.display())
        })?;
    }
    let preflight_script_path = sparse_focus_dir.join("preflight");

    let script_contents = r###"
    #!/bin/sh
    
    exec focus detect-build-graph-changes
"###;
    std::fs::write(&preflight_script_path, script_contents).with_context(|| {
        format!(
            "writing the preflight script to {}",
            &preflight_script_path.display()
        )
    })
}

fn configure_sparse_repo_final(
    dense_repo: &PathBuf,
    sparse_repo: &PathBuf,
    _branch: &str,
    sandbox: &Sandbox,
) -> Result<()> {
    // TODO: Figure out the remote based on the branch fetch/push config rather than assuming 'origin'. Kinda pedantic, but correct.
    let dense_git_dir = dense_repo.join(".git");
    let sparse_git_dir = sparse_repo.join(".git");

    let origin_journal_path = dense_git_dir
        .join("objects")
        .join("journals")
        .join("origin");
    let journal_state_lock_path = origin_journal_path.join("state.bin.lock");
    let _journal_state_lock =
        LockFile::new(&journal_state_lock_path).context("acquiring a lock on journal state")?;
    let sparse_journal_state_lock_path = sparse_git_dir
        .join("objects")
        .join("journals")
        .join("origin")
        .join("state.bin.lock");

    let paths_to_copy = vec![
        "config",
        "hooks",
        "hooks_multi",
        "repo.d",
        "objects/journals",
    ];
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

    if sparse_journal_state_lock_path.exists() {
        std::fs::remove_file(sparse_journal_state_lock_path)?;
    }

    configure_sparse_sync_point(sparse_repo, sandbox).context("configuring the sync point")?;

    setup_bazel_preflight_script(sparse_repo, sandbox).context("setting up build hooks")?;

    Ok(())
}

fn create_dense_link(dense_repo: &PathBuf, sparse_repo: &PathBuf) -> Result<()> {
    let link_path = sparse_repo.join(".dense");
    symlink(dense_repo, link_path).map_err(|e| Error::new(e))
}
// Write an object to a repo returning its identity.
fn git_hash_object(repo: &PathBuf, file: &PathBuf, sandbox: &Sandbox) -> Result<String> {
    git_helper::run_git_command_consuming_stdout(
        repo,
        vec![
            OsString::from("hash-object"),
            OsString::from("-w"),
            file.as_os_str().to_owned(),
        ],
        sandbox,
    )
}

fn git_remote_get_url(repo: &PathBuf, name: &str, sandbox: &Sandbox) -> Result<String> {
    git_helper::run_git_command_consuming_stdout(
        repo,
        vec![
            OsString::from("remote"),
            OsString::from("get-url"),
            OsString::from(name),
        ],
        sandbox,
    )
}

pub fn set_containing_layers(repo: &PathBuf, layer_names: &Vec<String>) -> Result<LayerSet> {
    let layer_sets = LayerSets::new(&repo);
    let rich_layer_set = RichLayerSet::new(
        layer_sets
            .available_layers()
            .context("getting available layers")?,
    )?;

    let mut layers = Vec::<Layer>::new();
    for layer_name in layer_names {
        if let Some(layer) = rich_layer_set.get(layer_name) {
            layers.push(layer.clone());
        } else {
            bail!("Layer named '{}' not found", &layer_name)
        }
    }

    Ok(LayerSet::new(layers.into_iter().collect()))
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
    dense_repo: &PathBuf,
    sparse_repo: &PathBuf,
    branch: &String,
    spec: &Spec,
    filter_sparse: bool,
    generate_project_view: bool,
    sandbox: Arc<Sandbox>,
) -> Result<()> {
    let mut adhoc_layer_set: Option<LayerSet> = None;

    let dense_sets = model::LayerSets::new(&dense_repo);
    let mut layer_set = dense_sets
        .mandatory_layers()
        .context("resolving mandatory layers")?;

    let user_selected_set = match spec {
        Spec::Coordinates(coordinates) => {
            // Put coordinates them into the "ad hoc" layer.
            let set = LayerSet::new(vec![Layer::new(
                "adhoc",
                "Ad hoc layer",
                false,
                coordinates,
            )]);
            adhoc_layer_set = Some(set.clone());
            set
        }

        Spec::Layers(layers) => {
            set_containing_layers(&dense_repo, layers).context("resolving user-selected layers")?
        }
    };

    // Check that the user selected set is valid
    user_selected_set
        .validate()
        .context("validating user-selected layers")?;

    // Add the user selected set to the overall set
    layer_set.extend(&user_selected_set);

    // Use coordinates_from_layers(repo, layer_names, sandbox)?

    let mut coordinates = Vec::<String>::new();
    for layer in layer_set.layers() {
        let layer_coordinates = layer.coordinates().clone();
        coordinates.extend(layer_coordinates);
    }

    create_or_update_sparse_clone(
        &dense_repo,
        &sparse_repo,
        &branch,
        &coordinates,
        filter_sparse,
        generate_project_view,
        true,
        sandbox,
    )?;

    if let Some(set) = &adhoc_layer_set {
        log::info!("Writing the adhoc layer set");
        write_adhoc_layer_set(&sparse_repo, set).context("writing the adhoc layer set")?;
    } else {
        // We know that layers were specified since there's no adhoc layer set.
        log::info!("Pushing the selected layers");
        let names = user_selected_set
            .layers()
            .iter()
            .map(|layer| layer.name().to_owned())
            .collect();
        let sparse_sets = LayerSets::new(&sparse_repo);
        sparse_sets
            .push_as_selection(names)
            .context("pushing the selected layers")?;
    }

    Ok(())
}

pub fn create_or_update_sparse_clone(
    dense_repo: &PathBuf,
    sparse_repo: &PathBuf,
    branch: &String,
    coordinates: &Vec<String>,
    filter_sparse: bool,
    generate_project_view: bool,
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

    let name: String = if let Some(name) = sparse_repo.file_name() {
        name.to_string_lossy().into()
    } else {
        bail!("unable to determine file stem for sparse repo directory");
    };

    let sparse_profile_output = sandbox.path().join("sparse-checkout");
    let project_view_output = {
        let project_view_name = format!("focus-{}.bazelproject", &name);
        sandbox.path().join(project_view_name)
    };

    configure_dense_repo(&dense_repo, sandbox.as_ref())
        .context("setting configuration options in the dense repo")?;

    // Make sure that the dense repo is in a clean state
    {
        let cloned_sandbox = sandbox.clone();
        let dense_sync = WorkingTreeSynchronizer::new(&dense_repo, cloned_sandbox.as_ref())
            .context("creating working tree synchronizer for dense repository")?;
        if !dense_sync
            .is_working_tree_clean()
            .context("determining dense repo working tree state")?
        {
            bail!("Dense repo has uncommitted changes");
        }
    }

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
                if !generate_project_view {
                    log::info!("Skipping generation of project view");
                    return;
                }

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
    set_up_alternates(&sparse_repo, &dense_repo).context("setting up an alternate")?;

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
        let cloned_branch = branch.clone();

        log::info!("Configuring visible paths");
        set_sparse_checkout(sparse_repo, &sparse_profile_output, &cloned_sandbox)
            .context("setting up sparse checkout options")?;

        log::info!("Finalizing configuration");
        if create {
            configure_sparse_repo_final(
                &cloned_dense_repo,
                &cloned_sparse_repo,
                &cloned_branch,
                &cloned_sandbox,
            )
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
        if project_view_output.is_file() {
            std::fs::rename(project_view_output, project_view_destination)
                .context("copying in the project view")?;
        }
    }

    Tracker::default()
        .ensure_registered(&sparse_repo, sandbox.as_ref())
        .context("adding sparse repo to the list of tracked repos")?;

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
        let _sparse_profile_destination = sparse_repo
            .join(".git")
            .join("info")
            .join("sparse-checkout");
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
