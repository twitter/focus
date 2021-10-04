use crate::app::App;
use crate::coordinate::CoordinateSet;
use crate::model::Layer;
use crate::model::LayerSets;
use crate::sandbox_command::SandboxCommand;
use std::convert::TryFrom;
use std::fs::File;

use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::{
    sandbox_command::SandboxCommandOutput, working_tree_synchronizer::WorkingTreeSynchronizer,
};

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

pub fn run(app: Arc<App>, sparse_repo: &Path) -> Result<()> {
    use crate::git_helper;
    use crate::sparse_repos;

    let ui = app.ui();
    let sparse_repo = git_helper::find_top_level(app.clone(), &sparse_repo)
        .context("canonicalizing sparse repo path")?;
    let dense_repo = git_helper::run_git_command_consuming_stdout(
        "Reading dense repo URL".to_owned(),
        &sparse_repo,
        vec!["remote", "get-url", "dense"],
        app.clone(),
    )
    .context("Failed reading the dense repo URL")
    .map(|path| PathBuf::from(path))?;
    let dense_repo = git_helper::find_top_level(app.clone(), &dense_repo)
        .context("Failed finding dense repo top level")?;

    let _ = ui.status(format!(
        "Syncing {}",
        &sparse_repo.display(),
    ));

    let sparse_sync = WorkingTreeSynchronizer::new(&sparse_repo, app.clone())?;
    let dense_sync = WorkingTreeSynchronizer::new(&dense_repo, app.clone())?;

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
            .mandatory_layers()
            .context("resolving mandatory layers")?;
        for layer in layer_set.layers() {
            merge_coordinates_from_layer(layer);
        }

        if let Some(selected) = sets.selected_layers().context("loading selected layers")? {
            // Add selected layers' coordinates
            if selected.layers().is_empty() {
                eprintln!("No layers are selected, but a stack exists");
                bail!("No layers found");
            }
            for layer in selected.layers() {
                merge_coordinates_from_layer(layer);
            }
        } else {
            // Add ad-hoc layer coordinates
            if let Some(adhoc_layers) = sets.adhoc_layers().context("reading adhoc layers")? {
                for layer in adhoc_layers.layers() {
                    merge_coordinates_from_layer(layer);
                }
            } else {
                // Fail because there are no selected layers or ad-hoc layer
                eprintln!("There are no selected layers and an ad-hoc layer does not exist.");
                eprintln!("The focused development working state in this repo might be corrupted.");
                bail!("No layers found");
            }
        }
        Ok(coordinates)
    })?;

    let coordinate_set =
        CoordinateSet::try_from(coordinates.as_ref()).context("constructing coordinate set")?;

    let cloned_app = app.clone();
    let dense_revision = perform("Determining the current commit in the dense repo", || {
        git_helper::run_git_command_consuming_stdout(
            "Determining the current commit in the dense repo".to_owned(),
            &dense_repo,
            vec!["rev-parse", "HEAD"],
            cloned_app,
        )
    })?;

    let cloned_app = app.clone();
    let sparse_revision = perform("Determining the current commit in the sparse repo", || {
        git_helper::run_git_command_consuming_stdout(
            "Determining the current commit in the sparse repo".to_owned(),
            &sparse_repo,
            vec!["rev-parse", "HEAD"],
            cloned_app,
        )
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

    let cloned_app = app.clone();
    perform("Switching in the dense repo", || {
        // When checking out in the dense repo, we make the sparse repo objects available as an alternate so as to not need to push to the dense repo.
        sparse_repos::switch_to_detached_branch_discarding_changes(
            &dense_repo,
            &sparse_revision.as_str(),
            Some(sparse_objects_directory.as_ref()),
            cloned_app,
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

    let cloned_app = app.clone();
    perform("Resetting in the dense repo", || {
        git_helper::run_git_command_consuming_stdout(
            "Resetting in the dense repo".to_owned(),
            &dense_repo,
            vec!["reset", "--hard", &dense_revision],
            cloned_app,
        )
    })?;

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

    let cloned_app = app.clone();
    perform("Disabling the filesystem monitor", || {
        sparse_repos::config_sparse_disable_filesystem_monitor(&sparse_repo, cloned_app)
    })?;

    Ok(())
}
