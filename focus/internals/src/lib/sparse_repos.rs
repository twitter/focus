use anyhow::{bail, Context, Result};
use tracing::debug;

use std::convert::TryFrom;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufWriter;
use std::os::unix::prelude::OsStrExt;
use std::path::Path;
use std::thread;
use std::thread::JoinHandle;
use std::{collections::BTreeSet, path::PathBuf, process::Stdio, sync::Arc};

use crate::app::App;
use crate::coordinate::CoordinateSet;
use crate::coordinate_resolver::CacheOptions;
use crate::coordinate_resolver::ResolutionRequest;
use crate::coordinate_resolver::Resolver;
use crate::coordinate_resolver::RoutingResolver;
use crate::model::layering::{Layer, LayerSet, LayerSets, RichLayerSet};
use crate::tracker::Tracker;
use crate::util::git_helper;
use crate::util::git_helper::run_consuming_stdout;
use crate::util::lock_file::LockFile;
use crate::util::paths;
use crate::util::sandbox_command::SandboxCommand;
use crate::util::sandbox_command::SandboxCommandOutput;
use crate::working_tree_synchronizer::WorkingTreeSynchronizer;

// TODO: Revisit this...
const SPARSE_PROFILE_PRELUDE: &str =
    "/tools\n/pants-plugins/\n/pants-support/\n/3rdparty/\n/focus/\n";

pub fn configure_dense_repo(dense_repo: &Path, app: Arc<App>) -> Result<()> {
    git_helper::write_config(dense_repo, "uploadPack.allowFilter", "true", app)
}

pub fn configure_sparse_repo_initial(_sparse_repo: &Path, _app: Arc<App>) -> Result<()> {
    Ok(())
}

fn set_up_alternates(sparse_repo: &Path, dense_repo: &Path) -> Result<()> {
    let alternates_path = sparse_repo
        .join(".git")
        .join("objects")
        .join("info")
        .join("alternates");
    let dense_odb = dense_repo.join(".git").join("objects");
    let dense_pruned_odb = dense_repo.join(".git").join("pruned-odb").join("objects");
    let sparse_pruned_odb = sparse_repo.join(".git").join("pruned-odb").join("objects");
    std::fs::create_dir_all(&sparse_pruned_odb).context("creating sparse pruned-odb")?;

    let mut buf = Vec::<u8>::new();
    if dense_odb.is_dir() {
        buf.extend(dense_odb.as_os_str().as_bytes());
        buf.push(b'\n');
    }
    if dense_pruned_odb.is_dir() {
        buf.extend(dense_pruned_odb.as_os_str().as_bytes());
        buf.push(b'\n');
    }
    buf.extend(sparse_pruned_odb.as_os_str().as_bytes());
    buf.push(b'\n');
    std::fs::write(alternates_path, buf).context("Failed to write the alternates file")?;

    Ok(())
}

// Set git config key focus.sync-point to HEAD
pub fn configure_sparse_sync_point(sparse_repo: &Path, app: Arc<App>) -> Result<()> {
    let head_str = git_helper::run_consuming_stdout(
        "Reading the current revision to use as a sync point".to_owned(),
        &sparse_repo,
        &["rev-parse", "HEAD"],
        app.clone(),
    )?;

    git_helper::write_config(sparse_repo, "focus.sync-point", head_str.as_str(), app)
}

// Disable filesystem monitor
pub fn config_sparse_disable_filesystem_monitor(sparse_repo: &Path, app: Arc<App>) -> Result<()> {
    git_helper::unset_config(sparse_repo, "core.fsmonitor", app)
}

// Set git config key focus.sync-point to HEAD
fn setup_bazel_preflight_script(sparse_repo: &Path) -> Result<()> {
    use std::os::unix::prelude::PermissionsExt;

    let sparse_focus_dir = sparse_repo.join(".focus");
    if !sparse_focus_dir.is_dir() {
        std::fs::create_dir(sparse_focus_dir.as_path()).with_context(|| {
            format!("failed to create directory {}", sparse_focus_dir.display())
        })?;
    }
    let preflight_script_path = sparse_focus_dir.join("preflight");
    {
        let mut preflight_script_file = BufWriter::new(
            File::create(preflight_script_path.as_path())
                .context("writing the build preflight script")?,
        );

        writeln!(preflight_script_file, "#!/bin/sh")?;
        writeln!(preflight_script_file)?;
        writeln!(
            preflight_script_file,
            "RUST_LOG=error exec focus detect-build-graph-changes"
        )?;
    }

    let mut perms = std::fs::metadata(preflight_script_path.as_path())
        .context("Reading permissions of the preflight script failed")?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(preflight_script_path, perms)
        .context("Setting permissions of the preflight script failed")?;

    Ok(())
}

fn create_branch(repo: &Path, ref_name: &str, commit_id: &str, app: Arc<App>) -> Result<()> {
    let cloned_app = app;
    if let Some(ref_name) = ref_name.strip_prefix("refs/heads/") {
        let description = format!("branch {} referencing commit {}", ref_name, commit_id);
        // Create the branch
        run_consuming_stdout(
            format!("Creating {}", description),
            repo,
            &["branch", ref_name, commit_id],
            cloned_app.clone(),
        )?;

        // Create the remote ref
        run_consuming_stdout(
            format!("Creating remote ref for {}", description),
            repo,
            &[
                "update-ref",
                &format!("refs/remotes/origin/{}", ref_name),
                commit_id,
            ],
            cloned_app.clone(),
        )?;

        // Set the branch's upsteam remote
        run_consuming_stdout(
            format!("Setting upstream remote for {}", description),
            repo,
            &["branch", "--set-upstream-to=origin", ref_name],
            cloned_app.clone(),
        )?;

        // Set the branch's upsteam merge ref
        run_consuming_stdout(
            format!("Setting upstream merge for {}", description),
            repo,
            &[
                "config",
                &format!("branch.{}.merge", ref_name),
                &format!("refs/heads/{}", ref_name),
            ],
            cloned_app,
        )?;
    } else {
        bail!(format!("Could not strip prefix from ref '{}'", ref_name));
    }
    Ok(())
}

fn copy_user_relevant_refs_to_sparse_repo(
    dense_repo: &Path,
    sparse_repo: &Path,
    branch: &str,
    app: Arc<App>,
) -> Result<()> {
    let cloned_app = app.clone();
    let ui = cloned_app.ui();

    let tokenize_ref_and_commit_ids = |line: &str| -> Result<(String, String)> {
        if let Some((ref_name, commit_id)) = line.split_once(' ') {
            Ok((ref_name.to_owned(), commit_id.to_owned()))
        } else {
            bail!(format!(
                "Failed to tokenize ref and commit ID from line '{}'",
                line
            ));
        }
    };

    let output = run_consuming_stdout(
        String::from("Retrieving the list of personal branches"),
        dense_repo,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads/",
        ],
        app,
    )
    .context(format!(
        "Failed to list refs in the dense repo ({})",
        dense_repo.display()
    ))?;

    let current_branch_with_prefix = format!("refs/heads/{}", branch);
    for line in output.lines() {
        match tokenize_ref_and_commit_ids(line) {
            Ok((ref_name, commit_id)) => {
                if ref_name == current_branch_with_prefix {
                    // Skip it.
                    ui.log("Ref Copy", format!("Skipping active ref {}", &ref_name));
                    continue;
                }

                create_branch(sparse_repo, &ref_name, &commit_id, cloned_app.clone())?;
            }
            Err(e) => bail!(e),
        }
    }

    Ok(())
}

fn configure_sparse_repo_final(
    dense_repo: &Path,
    sparse_repo: &Path,
    branch: &str,
    copy_branches: bool,
    app: Arc<App>,
) -> Result<()> {
    // TODO: Figure out the remote based on the branch fetch/push config rather than assuming 'origin'. Kinda pedantic, but correct.
    let dense_git_dir = dense_repo.join(".git");
    let sparse_git_dir = sparse_repo.join(".git");

    let origin_journal_path = dense_git_dir
        .join("objects")
        .join("journals")
        .join("origin");

    let _origin_journal_state_lock = {
        let journal_state_lock_path = origin_journal_path.join("state.bin.lock");
        if origin_journal_path.is_dir() {
            Some(
                LockFile::new(&journal_state_lock_path)
                    .context("acquiring a lock on journal state")?,
            )
        } else {
            None
        }
    };

    let sparse_journal_state_lock_path = sparse_git_dir
        .join("objects")
        .join("journals")
        .join("origin")
        .join("state.bin.lock");

    let paths_to_copy = vec![
        "config",
        "hooks",
        "hooks_multi",
        "objects/journals",
        "repo.d",
    ];

    for name in paths_to_copy {
        let app = app.clone();
        let from = dense_git_dir.join(name);
        if !from.exists() {
            continue;
        }
        // If the 'to' path is a directory, copy to its parent.
        let to = {
            let path = sparse_git_dir.join(name);
            if path.is_dir() {
                if let Some(parent) = path.parent() {
                    parent.to_owned()
                } else {
                    bail!(
                        "{} is a directory, however it has no parent",
                        path.display()
                    );
                }
            } else {
                path.to_owned()
            }
        };
        let description = format!("Copying {} -> {}", &from.display(), &to.display());
        let (mut cmd, scmd) = SandboxCommand::new(description.clone(), "cp", app)?;
        scmd.ensure_success_or_log(
            cmd.arg("-R").arg(&from).arg(&to),
            SandboxCommandOutput::Stderr,
            &description,
        )?;
    }

    git_helper::remote_add(&sparse_repo, "dense", dense_repo.as_os_str(), app.clone())
        .context("adding dense remote")?;

    if sparse_journal_state_lock_path.exists() {
        std::fs::remove_file(sparse_journal_state_lock_path)?;
    }

    if copy_branches {
        copy_user_relevant_refs_to_sparse_repo(dense_repo, sparse_repo, branch, app.clone())
            .context("Failed to copy branches to the sparse repo")?;
    }

    configure_sparse_sync_point(sparse_repo, app.clone())
        .context("Failed to set the sync point")?;

    setup_bazel_preflight_script(sparse_repo, app).context("Failed to set up build hooks")?;

    Ok(())
}

pub fn set_containing_layers(repo: &Path, layer_names: &[String]) -> Result<LayerSet> {
    let layer_sets = LayerSets::new(repo);
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

pub fn write_adhoc_layer_set(sparse_repo: &Path, layer_set: &LayerSet) -> Result<()> {
    let layer_sets = LayerSets::new(sparse_repo);
    layer_sets.store_adhoc_layers(layer_set)
}

pub fn create_sparse_clone(
    dense_repo: PathBuf,
    sparse_repo: PathBuf,
    branch: String,
    coordinates: Vec<String>,
    layers: Vec<String>,
    copy_branches: bool,
    app: Arc<App>,
) -> Result<()> {
    let dense_sets = LayerSets::new(&dense_repo);
    let mut layer_set = dense_sets
        .mandatory_layers()
        .context("Failed to resolve mandatory layers")?;

    // Add specified coordinates to an "ad-hoc" set
    let adhoc_set = LayerSet::new(vec![Layer::new(
        "adhoc",
        "Ad-hoc layer",
        false,
        coordinates,
    )]);

    // Add user selected layers
    let layer_backed_set =
        set_containing_layers(&dense_repo, &layers).context("resolving user-selected layers")?;

    // Check that the user selected set is valid
    layer_set.extend(adhoc_set.clone());
    layer_set.extend(layer_backed_set.clone());
    layer_set.validate().context("Failed to merged layer set")?;

    let coordinates: Vec<String> = layer_set
        .layers()
        .iter()
        .flat_map(|layer| layer.coordinates())
        .cloned()
        .collect();

    let cloned_app = app.clone();
    create_or_update_sparse_clone(
        &dense_repo,
        &sparse_repo,
        &branch,
        &coordinates,
        true,
        copy_branches,
        cloned_app,
    )?;

    // Write the ad-hoc set
    app.ui().log("Clone", "writing the ad-hoc layer set");
    write_adhoc_layer_set(&sparse_repo, &adhoc_set)
        .context("Failed writing the adhoc layer set")?;
    let layer_names: Vec<String> = layer_backed_set
        .layers()
        .iter()
        .map(|layer| layer.name().to_owned())
        .collect();
    app.ui()
        .log("Clone", "writing the stack of selected layers");
    LayerSets::new(&sparse_repo)
        .push_as_selection(layer_names)
        .context("Failed to write the selected layer set to the sparse repo")?;

    Ok(())
}

pub fn create_or_update_sparse_clone(
    dense_repo: &Path,
    sparse_repo: &Path,
    branch: &str,
    coordinates: &[String],
    create: bool,
    copy_branches: bool,
    app: Arc<App>,
) -> Result<()> {
    // TODO: Crash harder in threads to prevent extra work.
    let sandbox = app.sandbox();

    let coordinate_set =
        CoordinateSet::try_from(coordinates).context("formulating coordinate set failed")?;

    if create {
        if sparse_repo.is_dir() {
            bail!("Sparse repo already exists and creation was requested")
        }
    } else if !sparse_repo.is_dir() {
        bail!("Sparse repo does not exist and creation is not allowed")
    }

    let sparse_profile_output = sandbox.path().join("sparse-checkout");

    configure_dense_repo(dense_repo, app.clone())
        .context("setting configuration options in the dense repo")?;

    // Make sure that the dense repo is in a clean state
    {
        let cloned_app = app.clone();
        let dense_sync = WorkingTreeSynchronizer::new(dense_repo, cloned_app)
            .context("creating working tree synchronizer for dense repository")?;
        if !dense_sync
            .is_working_tree_clean()
            .context("determining dense repo working tree state")?
        {
            bail!("Dense repo has uncommitted changes")
        }
    }

    // Switch to the requested branch in the dense repo. Afterwards, we will switch back.
    let _dense_switch =
        git_helper::BranchSwitch::temporary(app.clone(), dense_repo, branch.to_owned(), None)?;

    let profile_generation_handle: JoinHandle<Result<()>> = {
        let cloned_app = app.clone();
        let cloned_dense_repo = dense_repo.to_owned();
        let cloned_sparse_profile_output = sparse_profile_output.to_owned();
        let cloned_coordinate_set = coordinate_set;

        thread::Builder::new()
            .name("SparseProfileGeneration".to_owned())
            .spawn(move || {
                cloned_app
                    .ui()
                    .log("Profile Generation", "Generating sparse profile");

                generate_sparse_profile(
                    &cloned_dense_repo,
                    &cloned_sparse_profile_output,
                    cloned_coordinate_set,
                    cloned_app.clone(),
                )
                .context("failed to generate a sparse profile")?;

                cloned_app
                    .ui()
                    .log("Profile Generation", "Finished generating sparse profile");

                Ok(())
            })
    }?;

    let clone_handle: JoinHandle<Result<()>> = {
        if !create {
            return Ok(());
        }

        let cloned_app = app.clone();
        let cloned_dense_repo = dense_repo.to_owned();
        let cloned_sparse_repo = sparse_repo.to_owned();
        let cloned_branch = branch.to_owned();

        thread::Builder::new()
            .name("CloneRepository".to_owned())
            .spawn(move || {
                cloned_app
                    .ui()
                    .log("Profile Generation", "Creating a template clone");
                create_empty_sparse_clone(
                    &cloned_dense_repo,
                    &cloned_sparse_repo,
                    &cloned_branch,
                    cloned_app.clone(),
                )
                .context("failed to create an empty sparse clone")?;
                configure_sparse_repo_initial(&cloned_sparse_repo, cloned_app.clone())
                    .context("failed to configure sparse clone")?;
                cloned_app
                    .ui()
                    .log("Profile Generation", "Finished creating a template clone");
                // N.B. For now, we set up alternates because they allow for journaled fetches
                set_up_alternates(&cloned_sparse_repo, &cloned_dense_repo)
                    .context("Setting up object database alternates failed")?;

                Ok(())
            })
    }?;

    // The clone has to finish before we can do anything else.
    if let Err(e) = clone_handle.join() {
        bail!("Cloning failed: {:?}", e);
    }

    if let Err(e) = profile_generation_handle.join() {
        bail!("Profile generation failed: {:?}", e);
    }

    {
        let cloned_app = app.clone();
        let cloned_sparse_repo = sparse_repo;
        let cloned_dense_repo = dense_repo;
        let cloned_branch = branch;

        cloned_app
            .ui()
            .log("Repository Setup", "Copying configuration");
        if create {
            configure_sparse_repo_final(
                cloned_dense_repo,
                cloned_sparse_repo,
                cloned_branch,
                copy_branches,
                cloned_app.clone(),
            )
            .context("failed to perform final configuration in the sparse repo")?;
        }

        cloned_app
            .ui()
            .log("Repository Setup", "Configuring visible paths");
        set_sparse_checkout(sparse_repo, &sparse_profile_output, cloned_app.clone())
            .context("Failed to set the sparse checkout file")?;

        cloned_app
            .ui()
            .log("Repository Setup", "Checking out the working copy");
        checkout_working_copy(cloned_sparse_repo, cloned_app.clone())
            .context("Failed to check out the working copy")?;
        cloned_app
            .ui()
            .log("Repository Setup", "Setting up other branches");

        cloned_app
            .ui()
            .log("Repository Setup", "Moving the project view into place");
    }

    Tracker::default()
        .ensure_registered(sparse_repo, app)
        .context("adding sparse repo to the list of tracked repos")?;

    Ok(())
}

pub fn set_sparse_config(sparse_repo: &Path, app: Arc<App>) -> Result<()> {
    git_helper::write_config(&sparse_repo, "core.sparseCheckout", "true", app.clone())?;
    git_helper::write_config(&sparse_repo, "core.sparseCheckoutCone", "true", app)?;
    Ok(())
}

pub fn set_sparse_checkout(sparse_repo: &Path, sparse_profile: &Path, app: Arc<App>) -> Result<()> {
    set_sparse_config(sparse_repo, app.clone())?;
    {
        // TODO: If the git version supports it, add --no-sparse-index since the sparse index performs poorly
        let (mut cmd, scmd) =
            git_helper::git_command("Initializing baseline sparse checkout", app.clone())?;
        scmd.ensure_success_or_log(
            cmd.current_dir(sparse_repo)
                .arg("sparse-checkout")
                .arg("init")
                .arg("--cone"),
            SandboxCommandOutput::Stderr,
            "sparse-checkout init",
        )
        .map(|_| ())
        .context("Failed to initialize the sparse checkout")?;
    }

    {
        // TODO: If the git version supports it, add --no-sparse-index since the sparse index performs poorly
        let sparse_profile_file = File::open(&sparse_profile).context("opening sparse profile")?;
        let (mut cmd, scmd) = SandboxCommand::new_with_handles(
            "Adding directories".to_owned(),
            git_helper::git_binary(),
            Some(Stdio::from(sparse_profile_file)),
            None,
            None,
            app,
        )?;
        scmd.ensure_success_or_log(
            cmd.current_dir(sparse_repo)
                .arg("sparse-checkout")
                .arg("set")
                .arg("--stdin"),
            SandboxCommandOutput::Stderr,
            "sparse-checkout add",
        )
        .map(|_| ())
        .context("Failed to set the sparse checkout")?;
    }

    Ok(())
}

pub fn checkout_working_copy(sparse_repo: &Path, app: Arc<App>) -> Result<()> {
    // TODO: If the git version supports it, add --no-sparse-index since the sparse index performs poorly
    let (mut cmd, scmd) = git_helper::git_command("Checking out a working copy", app)?;
    scmd.ensure_success_or_log(
        cmd.current_dir(sparse_repo).arg("checkout"),
        SandboxCommandOutput::Stderr,
        "checkout",
    )
    .map(|_| ())
    .context("checking out the working copy")
}

pub fn create_empty_sparse_clone(
    dense_repo: &Path,
    sparse_repo: &Path,
    branch: &str,
    app: Arc<App>,
) -> Result<()> {
    let _ui = app.ui();

    let sparse_repo_dir_parent = &sparse_repo
        .parent()
        .context("sparse repo parent directory does not exist")?;

    // TODO: If the git version supports it, add --no-sparse-index since the sparse index performs poorly
    let description = format!(
        "Creating a new a sparse shallow clone of {} in {}",
        dense_repo.display(),
        sparse_repo.display()
    );
    let (mut cmd, scmd) = git_helper::git_command(description, app)?;
    scmd.ensure_success_or_log(
        cmd.current_dir(sparse_repo_dir_parent)
            .arg("clone")
            .arg("--local")
            .arg("--shared")
            .arg("--no-checkout")
            .arg("--no-tags")
            .arg("--single-branch")
            .arg("-b")
            .arg(branch)
            .arg(dense_repo)
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
        writeln!(buffer)?;
        buffer.flush()?;
    }

    Ok(())
}

fn resolve_involved_directories<P: AsRef<Path> + std::fmt::Debug>(
    repo: P,
    coordinate_set: CoordinateSet,
    app: Arc<App>,
    into: &mut BTreeSet<PathBuf>,
) -> Result<usize> {
    let repo = repo.as_ref();
    let cache_dir = dirs::cache_dir()
        .context("failed to determine cache dir")?
        .join("focus")
        .join("cache");
    let resolver = RoutingResolver::new(cache_dir.as_path());

    let request = ResolutionRequest {
        repo: repo.to_owned(),
        coordinate_set,
    };
    let cache_options = CacheOptions::default();

    let result = resolver
        .resolve(&request, &cache_options, app.clone())
        .context("Failed to resolve coordinates")?;

    let before = into.len();
    let total = result.paths.len();
    for path in result.paths {
        let qualified_path = repo.join(path);
        if let Some(path_to_closest_build_file) =
            paths::find_closest_directory_with_build_file(&qualified_path, repo)
                .context("locating closest build file")?
        {
            debug!(
                ?path_to_closest_build_file,
                "Adding directory with closest build definition",
            );
            into.insert(path_to_closest_build_file);
        } else {
            debug!(?qualified_path, "Adding directory verbatim");
            into.insert(qualified_path.to_owned());
        }
    }

    let difference = into.len() - before;
    app.ui().log(
        String::from("Resolver"),
        format!(
            "Resolution yielded {} directories ({} total)",
            difference, total,
        ),
    );

    Ok(difference)
}

pub fn generate_sparse_profile(
    repo: &Path,
    sparse_profile_output: &Path,
    coordinate_set: CoordinateSet,
    app: Arc<App>,
) -> Result<()> {
    let mut directories = BTreeSet::<PathBuf>::new();

    let mut f = File::create(&sparse_profile_output).context("creating output file")?;
    f.write_all(SPARSE_PROFILE_PRELUDE.as_bytes())
        .context("writing sparse profile prelude")?;
    resolve_involved_directories(repo, coordinate_set, app, &mut directories)
        .context("resolving involved directories")?;

    for dir in &directories {
        let mut line = Vec::<u8>::new();
        line.extend(b"/"); // Paths have a '/' prefix
        {
            let dir = dir
                .as_path()
                .strip_prefix(repo)
                .context("Failed to strip prefix")?;
            debug!(?dir, "Adding directory");
            line.extend(dir.as_os_str().as_bytes());
        }
        line.extend(b"/\n"); // Paths have a '/' suffix
        f.write_all(&line[..]).context("writing paths")?;
    }
    f.sync_data().context("syncing data")?;

    Ok(())
}
