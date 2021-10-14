use crate::app::App;
use crate::coordinate::CoordinateSet;
use crate::git_helper;
use crate::git_helper::get_current_revision;
use crate::git_helper::BranchSwitch;
use crate::model::Layer;
use crate::model::LayerSets;
use crate::util::sandbox_command::SandboxCommand;
use crate::util::sandbox_command::SandboxCommandOutput;
use crate::working_tree_synchronizer::WorkingTreeSynchronizer;
use std::convert::TryFrom;
use std::fs::File;

use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

pub fn perform<F, J>(description: &str, f: F) -> Result<J>
where
    F: FnOnce() -> Result<J>,
{
    let result = f();
    if let Err(e) = result {
        log::error!("Failed {}: {}", description.to_ascii_lowercase(), e);
        bail!(e);
    }

    result
}

fn find_dense_repo(app: Arc<App>, sparse_repo: &Path) -> Result<PathBuf> {
    let dense_repo = git_helper::run_consuming_stdout(
        "Reading dense repo URL".to_owned(),
        &sparse_repo,
        vec!["remote", "get-url", "dense"],
        app.clone(),
    )
    .context("Failed reading the dense repo URL")
    .map(|path| PathBuf::from(path))?;
    let dense_repo = git_helper::find_top_level(app.clone(), &dense_repo)
        .context("Failed finding dense repo top level")?;
    Ok(dense_repo)
}

pub fn ensure_working_trees_are_clean(
    app: Arc<App>,
    sparse_repo: &Path,
    dense_repo: Option<PathBuf>,
) -> Result<()> {
    let sparse_repo = git_helper::find_top_level(app.clone(), &sparse_repo)
        .context("canonicalizing sparse repo path")?;
    let dense_repo = {
        if let Some(dense_repo) = dense_repo {
            dense_repo
        } else {
            find_dense_repo(app.clone(), &sparse_repo)?
        }
    };
    let sparse_sync = WorkingTreeSynchronizer::new(&sparse_repo, app.clone())?;
    let dense_sync = WorkingTreeSynchronizer::new(&dense_repo, app.clone())?;

    if let Ok(clean) = perform("Checking that dense repo is in a clean state", || {
        dense_sync.is_working_tree_clean()
    }) {
        if !clean {
            eprintln!("The working tree in the dense repo must be in a clean state. Commit or stash changes and try to run the sync again.");
            bail!("Dense repo working tree is not in a clean state");
        }
    } else {
        bail!("Could not determine whether the dense repo is in a clean state");
    }

    if let Ok(clean) = perform("Checking that sparse repo is in a clean state", || {
        sparse_sync.is_working_tree_clean()
    }) {
        if !clean {
            eprintln!("The working tree in the sparse repo must be in a clean state. Commit or stash changes and try to run the sync again.");
            bail!("Sparse repo working tree is not in a clean state");
        }
    } else {
        bail!("Could not determine whether the sparse repo is in a clean state");
    }

    Ok(())
}

pub fn run(app: Arc<App>, sparse_repo: &Path) -> Result<()> {
    // TODO(wilhelm): Make this multi-threaded where possible.
    use crate::sparse_repos;
    let ui = app.ui();
    let sparse_repo = git_helper::find_top_level(app.clone(), &sparse_repo)
        .context("canonicalizing sparse repo path")?;
    let dense_repo = find_dense_repo(app.clone(), &sparse_repo)?;

    let sparse_profile_path = sparse_repo
        .join(".git")
        .join("info")
        .join("sparse-checkout");

    let (sparse_profile_output_file, sparse_profile_output_path) =
        app.sandbox().create_file(Some("sparse-profile"), None)?;
    drop(sparse_profile_output_file);

    let sparse_checkout_backup_path = {
        let mut path = sparse_profile_path.clone();
        path.set_extension("backup");
        path
    };

    ui.status(format!("Syncing {}", &sparse_repo.display()));

    ensure_working_trees_are_clean(app.clone(), sparse_repo.as_path(), Some(dense_repo.clone()))
        .context("Failed trying to determine whether working trees were clean")?;

    // Figure out all of the coordinates we will be resolving
    let coordinates = perform("Enumerating coordinates", || {
        let mut coordinates = Vec::<String>::new();
        let mut merge_coordinates_from_layer = |layer: &Layer| {
            let coordinates_in_layer: Vec<String> = layer
                .coordinates()
                .iter()
                .map(|coord| coord.to_owned())
                .collect::<_>();
            coordinates.extend(coordinates_in_layer);
        };

        // Add mandatory layers
        let sets = LayerSets::new(&sparse_repo);
        let layer_set = sets
            .computed_layers()
            .context("Failed resolving applied layers")?;
        for layer in layer_set.layers() {
            merge_coordinates_from_layer(layer);
        }

        Ok(coordinates)
    })?;

    let coordinate_set =
        CoordinateSet::try_from(coordinates.as_ref()).context("constructing coordinate set")?;

    let cloned_app = app.clone();
    let sparse_revision = perform("Determining the current commit in the sparse repo", || {
        get_current_revision(cloned_app, sparse_repo.as_path())
    })?;

    perform("Backing up the current sparse checkout file", || {
        std::fs::copy(&sparse_profile_path, &sparse_checkout_backup_path)
            .context("copying to the backup file")?;

        Ok(())
    })?;

    let sparse_objects_directory = std::fs::canonicalize(&sparse_repo)
        .context("canonicalizing sparse path")?
        .join(".git")
        .join("objects");

    {
        let cloned_app = app.clone();
        let _dense_switch = perform("Switching in the dense repo", || {
            // When checking out in the dense repo, we make the sparse repo objects available as an alternate so as to not need to push to the dense repo.
            BranchSwitch::temporary(
                cloned_app,
                dense_repo.clone(),
                sparse_revision,
                Some(sparse_objects_directory.clone()),
            )
        })?;

        let cloned_app = app.clone();
        perform("Computing the new sparse profile", || {
            sparse_repos::generate_sparse_profile(
                &dense_repo,
                &sparse_profile_output_path,
                coordinate_set,
                cloned_app,
            )
        })?;

        // The dense switch will go out of scope and that repo will switch back.
    }

    let cloned_app = app.clone();
    if let Err(_e) = perform("Applying the sparse profile", || {
        sparse_repos::set_sparse_config(&sparse_repo, cloned_app.clone())?;

        let sparse_profile_output_file =
            File::open(&sparse_profile_output_path).context("opening new sparse profile")?;
        let (mut cmd, scmd) = SandboxCommand::new_with_handles(
            "Applying the sparse profile".to_owned(),
            git_helper::git_binary(),
            Some(Stdio::from(sparse_profile_output_file)),
            None,
            None,
            cloned_app.clone(),
        )?;
        scmd.ensure_success_or_log(
            cmd.current_dir(&sparse_repo)
                .arg("sparse-checkout")
                .arg("set")
                .arg("--stdin"),
            SandboxCommandOutput::Stderr,
            "sparse-checkout set",
        )
        .map(|_| ())
        .context("setting sparse checkout from new profile")
    }) {
        perform("Restoring and reapplying the backup profile", || {
            let backup_file = File::open(&sparse_checkout_backup_path)
                .context("opening backup sparse profile")?;
            let (mut cmd, scmd) = SandboxCommand::new_with_handles(
                "Restoring and reapplying the backup profile".to_owned(),
                git_helper::git_binary(),
                Some(Stdio::from(backup_file)),
                None,
                None,
                cloned_app.clone(),
            )?;
            scmd.ensure_success_or_log(
                cmd.current_dir(&sparse_repo)
                    .arg("sparse-checkout")
                    .arg("set")
                    .arg("--cone")
                    .arg("--stdin"),
                SandboxCommandOutput::Stderr,
                "sparse-checkout set",
            )
            .map(|_| ())
        })?;
    }

    let cloned_app = app.clone();
    perform("Updating the sync point", || {
        sparse_repos::configure_sparse_sync_point(&sparse_repo, cloned_app)
    })?;

    Ok(())
}
