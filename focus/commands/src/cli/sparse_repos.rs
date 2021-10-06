use anyhow::{bail, Context, Result};
use internals::util::lock_file::LockFile;

use std::convert::TryFrom;
use std::ffi::OsString;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufWriter;
use std::os::unix::prelude::OsStrExt;
use std::path::Path;
use std::thread;
use std::{collections::BTreeSet, path::PathBuf, process::Stdio, sync::Arc};

use crate::app::App;
use crate::coordinate::CoordinateSet;
use crate::coordinate_resolver::CacheOptions;
use crate::coordinate_resolver::RepoState;
use crate::coordinate_resolver::ResolutionRequest;
use crate::coordinate_resolver::Resolver;
use crate::coordinate_resolver::RoutingResolver;
use crate::git_helper;
use crate::model::{Layer, LayerSet, LayerSets, RichLayerSet};
use crate::sandbox_command::SandboxCommand;
use crate::sandbox_command::SandboxCommandOutput;
use crate::tracker::Tracker;
use crate::ui::ProgressReporter;
use crate::working_tree_synchronizer::WorkingTreeSynchronizer;

// TODO: Revisit this...
const SPARSE_PROFILE_PRELUDE: &str =
    "/tools\n/pants-plugins/\n/pants-support/\n/3rdparty/\n/focus/\n";

fn report_progress<T>(app: Arc<App>, message: T) -> Result<ProgressReporter>
where
    T: AsRef<str>,
{
    Ok(ProgressReporter::new(
        app.clone(),
        message.as_ref().to_string(),
    )?)
}

pub fn configure_dense_repo(dense_repo: &PathBuf, app: Arc<App>) -> Result<()> {
    git_helper::write_config(dense_repo, "uploadPack.allowFilter", "true", app)
}

pub fn configure_sparse_repo_initial(_sparse_repo: &PathBuf, _app: Arc<App>) -> Result<()> {
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
    let head_str = git_helper::run_git_command_consuming_stdout(
        "Reading the current revision to use as a sync point".to_owned(),
        &sparse_repo,
        vec!["rev-parse", "HEAD"],
        app.clone(),
    )?;

    git_helper::write_config(sparse_repo, "focus.sync-point", head_str.as_str(), app)
}

// Disable filesystem monitor
pub fn config_sparse_disable_filesystem_monitor(sparse_repo: &Path, app: Arc<App>) -> Result<()> {
    git_helper::unset_config(sparse_repo, "core.fsmonitor", app)
}

// Set git config key focus.sync-point to HEAD
fn setup_bazel_preflight_script(sparse_repo: &PathBuf, _app: Arc<App>) -> Result<()> {
    let sparse_focus_dir = sparse_repo.join(".focus");
    if !sparse_focus_dir.is_dir() {
        std::fs::create_dir(sparse_focus_dir.as_path()).with_context(|| {
            format!("failed to create directory {}", sparse_focus_dir.display())
        })?;
    }
    let preflight_script_path = sparse_focus_dir.join("preflight");
    let mut preflight_script_file = BufWriter::new(
        File::create(preflight_script_path).context("writing the build preflight script")?,
    );

    writeln!(preflight_script_file, "#!/bin/sh")?;
    writeln!(preflight_script_file, "")?;
    writeln!(
        preflight_script_file,
        "exec focus detect-build-graph-changes"
    )?;

    Ok(())
}

fn configure_sparse_repo_final(
    dense_repo: &PathBuf,
    sparse_repo: &PathBuf,
    _branch: &str,
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
        "repo.d",
        "objects/journals",
    ];

    for name in paths_to_copy {
        let app = app.clone();
        let from = dense_git_dir.join(name);
        if !from.exists() {
            continue;
        }
        let to = sparse_git_dir.join(name);
        let description = format!("Copying {} -> {}", &from.display(), &to.display());
        let (mut cmd, scmd) = SandboxCommand::new(description.clone(), "cp", app)?;
        scmd.ensure_success_or_log(
            cmd.arg("-r").arg(&from).arg(&to),
            SandboxCommandOutput::Stderr,
            &description,
        )?;
    }

    git_helper::remote_add(&sparse_repo, "dense", dense_repo.as_os_str(), app.clone())
        .context("adding dense remote")?;

    if sparse_journal_state_lock_path.exists() {
        std::fs::remove_file(sparse_journal_state_lock_path)?;
    }

    configure_sparse_sync_point(sparse_repo, app.clone()).context("configuring the sync point")?;

    setup_bazel_preflight_script(sparse_repo, app.clone()).context("setting up build hooks")?;

    Ok(())
}

// Write an object to a repo returning its identity.
fn git_hash_object(repo: &PathBuf, file: &PathBuf, app: Arc<App>) -> Result<String> {
    git_helper::run_git_command_consuming_stdout(
        format!("Writing {} to the object store", file.display()),
        repo,
        vec![
            OsString::from("hash-object"),
            OsString::from("-w"),
            file.as_os_str().to_owned(),
        ],
        app,
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

pub fn create_sparse_clone(
    dense_repo: PathBuf,
    sparse_repo: PathBuf,
    branch: String,
    coordinates: Vec<String>,
    layers: Vec<String>,
    app: Arc<App>,
) -> Result<()> {
    let dense_sets = LayerSets::new(&dense_repo);
    let mut layer_set = dense_sets
        .mandatory_layers()
        .context("Failed to resolve  mandatory layers")?;

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

    let mut coordinates = Vec::<String>::new();
    for layer in layer_set.layers() {
        let layer_coordinates = layer.coordinates().clone();
        coordinates.extend(layer_coordinates);
    }

    let cloned_app = app.clone();
    create_or_update_sparse_clone(
        &dense_repo,
        &sparse_repo,
        &branch,
        &coordinates,
        true,
        cloned_app,
    )?;

    // Write the ad-hoc set
    let _ = app.ui().log(
        String::from("Clone"),
        String::from("writing the ad-hoc layer set"),
    );
    write_adhoc_layer_set(&sparse_repo, &adhoc_set)
        .context("Failed writing the adhoc layer set")?;
    let layer_names: Vec<String> = layer_backed_set
        .layers()
        .iter()
        .map(|layer| layer.name().to_owned())
        .collect();
    let _ = app.ui().log(
        String::from("Clone"),
        String::from("writing the stack of selected layers"),
    );
    LayerSets::new(&sparse_repo)
        .push_as_selection(layer_names)
        .context("Failed to write the selected layer set to the sparse repo")?;

    Ok(())
}

pub fn create_or_update_sparse_clone(
    dense_repo: &PathBuf,
    sparse_repo: &PathBuf,
    branch: &String,
    coordinates: &Vec<String>,
    create: bool,
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

    configure_dense_repo(&dense_repo, app.clone())
        .context("setting configuration options in the dense repo")?;

    // Make sure that the dense repo is in a clean state
    {
        let cloned_app = app.clone();
        let dense_sync = WorkingTreeSynchronizer::new(&dense_repo, cloned_app)
            .context("creating working tree synchronizer for dense repository")?;
        if !dense_sync
            .is_working_tree_clean()
            .context("determining dense repo working tree state")?
        {
            bail!("Dense repo has uncommitted changes");
        }
    }

    // Being on the right branch in the dense repository is a prerequisite for any work.
    switch_to_detached_branch_discarding_changes(&dense_repo, &branch, None, app.clone())?;

    let profile_generation_handle = {
        let cloned_app = app.clone();
        let cloned_dense_repo = dense_repo.to_owned();
        let cloned_sparse_profile_output = sparse_profile_output.to_owned();
        let cloned_coordinate_set = coordinate_set.clone();

        thread::Builder::new()
            .name("SparseProfileGeneration".to_owned())
            .spawn(move || {
                let _ = cloned_app.ui().log(
                    String::from("Profile Generation"),
                    String::from("Generating sparse profile"),
                );

                generate_sparse_profile(
                    &cloned_dense_repo,
                    &cloned_sparse_profile_output,
                    cloned_coordinate_set,
                    cloned_app.clone(),
                )
                .expect("failed to generate a sparse profile");

                let _ = cloned_app.ui().log(
                    String::from("Profile Generation"),
                    String::from("Finished generating sparse profile"),
                );
            })
    }?;

    let clone_handle = {
        if !create {
            return Ok(());
        }

        let cloned_app = app.clone();
        let cloned_dense_repo = dense_repo.to_owned();
        let cloned_sparse_repo = sparse_repo.to_owned();
        let cloned_branch = branch.clone();

        thread::Builder::new()
            .name("CloneRepository".to_owned())
            .spawn(move || {
                let _ = cloned_app.ui().log(
                    String::from("Profile Generation"),
                    String::from("Creating a template clone"),
                );
                create_empty_sparse_clone(
                    &cloned_dense_repo,
                    &cloned_sparse_repo,
                    &cloned_branch,
                    cloned_app.clone(),
                )
                .expect("failed to create an empty sparse clone");
                configure_sparse_repo_initial(&cloned_sparse_repo, cloned_app.clone())
                    .expect("failed to configure sparse clone");
                let _ = cloned_app.ui().log(
                    String::from("Profile Generation"),
                    String::from("Finished creating a template clone"),
                );
                // N.B. For now, we set up alternates because they allow for journaled fetches
                set_up_alternates(&cloned_sparse_repo, &cloned_dense_repo)
                    .expect("Setting up object database alternates failed");
            })
    }?;

    // The clone has to finish before we can do anything else.
    if let Err(e) = clone_handle.join() {
        bail!("Cloning failed: {:?}", e);
    }

    if let Err(e) = profile_generation_handle.join() {
        bail!("Profile Generation failed: {:?}", e);
    }

    {
        let cloned_app = app.clone();
        let cloned_sparse_repo = sparse_repo.clone();
        let cloned_dense_repo = dense_repo.clone();
        let cloned_branch = branch.clone();

        let _ = cloned_app.ui().log(
            String::from("Repository Setup"),
            String::from("Copying configuration"),
        );
        if create {
            configure_sparse_repo_final(
                &cloned_dense_repo,
                &cloned_sparse_repo,
                &cloned_branch,
                cloned_app.clone(),
            )
            .context("failed to perform final configuration in the sparse repo")?;
        }

        let _ = cloned_app.ui().log(
            String::from("Repository Setup"),
            String::from("Configuring visible paths"),
        );
        set_sparse_checkout(sparse_repo, &sparse_profile_output, cloned_app.clone())
            .context("Failed to set the sparse checkout file")?;

        let _ = cloned_app.ui().log(
            String::from("Repository Setup"),
            String::from("Checking out the working copy"),
        );
        checkout_working_copy(&cloned_sparse_repo, cloned_app.clone())
            .context("Failed to check out the working copy")?;
        let _ = cloned_app.ui().log(
            String::from("Repository Setup"),
            String::from("Setting up other branches"),
        );

        let _ = cloned_app.ui().log(
            String::from("Repository Setup"),
            String::from("Moving the project view into place"),
        );
    }

    Tracker::default()
        .ensure_registered(&sparse_repo, app)
        .context("adding sparse repo to the list of tracked repos")?;

    Ok(())
}

pub fn set_sparse_config(sparse_repo: &Path, app: Arc<App>) -> Result<()> {
    git_helper::write_config(&sparse_repo, "core.sparseCheckout", "true", app.clone())?;
    git_helper::write_config(&sparse_repo, "core.sparseCheckoutCone", "true", app.clone())?;
    Ok(())
}

pub fn set_sparse_checkout(
    sparse_repo: &PathBuf,
    sparse_profile: &PathBuf,
    app: Arc<App>,
) -> Result<()> {
    set_sparse_config(&sparse_repo, app.clone())?;
    {
        // TODO: If the git version supports it, add --no-sparse-index since the sparse index performs poorly
        let (mut cmd, scmd) = git_helper::git_command(
            "Initializing sparse checkout profile".to_owned(),
            app.clone(),
        )?;
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
        .context("initializing sparse checkout")?;
    }

    Ok(())
}

pub fn checkout_working_copy(sparse_repo: &PathBuf, app: Arc<App>) -> Result<()> {
    // TODO: If the git version supports it, add --no-sparse-index since the sparse index performs poorly
    let (mut cmd, scmd) = git_helper::git_command("Checking out a working copy".to_owned(), app)?;
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
    app: Arc<App>,
) -> Result<()> {
    let app = app.clone();
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
    let (mut cmd, scmd) = git_helper::git_command(description, app.clone())?;
    scmd.ensure_success_or_log(
        cmd.current_dir(sparse_repo_dir_parent)
            .arg("clone")
            .arg("--sparse")
            .arg("--local")
            .arg("--no-checkout")
            .arg("--no-tags")
            .arg("--single-branch")
            .arg("-b")
            .arg(branch.as_str())
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
    _dense_repo: &PathBuf,
    _bazel_project_view_path: &Path,
    _coordinates: &Vec<String>,
    _app: Arc<App>,
) -> Result<()> {
    // TODO: Remove this. We will implement a 'focus idea' command or similar.
    Ok(())
}

pub fn switch_to_detached_branch_discarding_changes(
    repo: &Path,
    refname: &str,
    alternate: Option<&Path>,
    app: Arc<App>,
) -> Result<()> {
    let description = format!("Switching to {} in {}", refname, repo.display());
    let (mut cmd, scmd) = git_helper::git_command(description, app)?;
    let cmd = cmd
        .arg("switch")
        .arg(refname)
        .arg("--quiet")
        .arg("--detach")
        .arg("--discard-changes")
        .current_dir(&repo);
    if let Some(alternate_path) = alternate {
        cmd.env(
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            alternate_path.as_os_str(),
        );
    }
    scmd.ensure_success_or_log(
        cmd,
        SandboxCommandOutput::Stderr,
        &format!("switching to ref '{}' in repo {}", refname, &repo.display()),
    )?;

    Ok(())
}

fn resolve_involved_directories(
    repo: &Path,
    coordinate_set: CoordinateSet,
    app: Arc<App>,
    into: &mut BTreeSet<PathBuf>,
) -> Result<usize> {
    let cache_dir = dirs::cache_dir()
        .context("failed to determine cache dir")?
        .join("focus")
        .join("cache");
    let resolver = RoutingResolver::new(cache_dir.as_path());

    let repo_state = RepoState::new(&repo, app.clone())?;
    let request = ResolutionRequest::new(repo, repo_state, coordinate_set);
    let cache_options = CacheOptions::default();

    let result = resolver
        .resolve(&request, &cache_options, app.clone())
        .context("resolving coordinates")?;

    let before = into.len();
    for path in result.paths() {
        let qualified_path = repo.join(path);
        if let Some(path) = find_closest_directory_with_build_file(&qualified_path, repo)
            .context("locating closest build file")?
        {
            into.insert(path);
        }
    }

    let difference = into.len() - before;
    report_progress(
        app,
        format!(
            "Dependency query yielded {} directories ({} total)",
            difference,
            &result.paths().len()
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
    f.write_all(&SPARSE_PROFILE_PRELUDE.as_bytes())
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
            log::debug!("+ {}", &dir.display());
            line.extend(dir.as_os_str().as_bytes());
        }
        line.extend(b"/\n"); // Paths have a '/' suffix
        f.write_all(&line[..]).context("writing paths")?;
    }
    f.sync_data().context("syncing data")?;

    Ok(())
}

fn find_closest_directory_with_build_file(file: &Path, ceiling: &Path) -> Result<Option<PathBuf>> {
    let mut dir = if file.is_dir() {
        file
    } else {
        if let Some(parent) = file.parent() {
            parent
        } else {
            log::warn!("Path {} has no parent", file.display());
            return Ok(None);
        }
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
