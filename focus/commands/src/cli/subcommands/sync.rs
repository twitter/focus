use crate::model::Layer;
use crate::model::LayerSets;
use crate::sandbox_command::SandboxCommand;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Context, Result};

use crate::{
    sandbox::Sandbox, sandbox_command::SandboxCommandOutput,
    working_tree_synchronizer::WorkingTreeSynchronizer,
};

pub fn perform<F, J>(description: &str, f: F) -> Result<J>
where
    F: FnOnce() -> Result<J>,
{
    log::debug!("Task started: {}", description);
    let result = f();
    if let Err(e) = result {
        log::error!("Task failed: {}: {}", description, e);
        bail!(e);
    }
    log::info!("Task succeeded: {}", description);

    result
}

pub fn run(sandbox: &Sandbox, repo: &Path) -> Result<()> {
    use crate::git_helper;
    use crate::sparse_repos;

    let sparse_sync = WorkingTreeSynchronizer::new(&repo, &sandbox)?;
    let sparse_profile_path = repo.join(".git").join("info").join("sparse-checkout");

    let (sparse_profile_output_file, sparse_profile_output_path) =
        sandbox.create_file(Some("sparse-profile"), None)?;
    drop(sparse_profile_output_file);

    let sparse_checkout_backup_path = {
        let mut path = sparse_profile_path.clone();
        path.set_extension("backup");
        path
    };

    if let Ok(clean) = perform("Checking that sparse repo is in a clean state", || {
        sparse_sync.is_working_tree_clean()
    }) {
        if !clean {
            eprintln!("The working tree must be in a clean state. Commit or stash changes and try to run the sync again.");
            bail!("Working tree is not clean");
        }
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
        let sets = LayerSets::new(&repo);
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
            if let Ok(Some(adhoc_layers)) = sets.adhoc_layers() {
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

    perform("Backing up the current sparse checkout file", || {
        std::fs::copy(&sparse_profile_path, &sparse_checkout_backup_path)
            .context("copying to the backup file")?;

        Ok(())
    })?;

    perform("Computing the new sparse profile", || {
        sparse_repos::generate_sparse_profile(
            &repo,
            &sparse_profile_output_path,
            &coordinates,
            &sandbox,
        )
    })?;

    let merged_output_path = {
        let mut path = sparse_profile_output_path.clone();
        path.set_extension("merged");
        path
    };

    perform("Converting the new sparse profile to cone patterns", || {
        use std::io::BufRead;
        use std::io::Write;

        // Merge everything together in the way that Git likes it for cone patterns because we can't use add/init.
        {
            let mut merged_output_file =
                File::create(&merged_output_path).context("opening merged output file")?;
            writeln!(merged_output_file, "/*")?;
            writeln!(merged_output_file, "!/*/")?;
            let sparse_profile_output_file =
                BufReader::new(File::open(sparse_profile_output_path)?);
            for line in sparse_profile_output_file.lines() {
                if let Ok(line) = line {
                    let trimmed = line.trim();
                    if trimmed.eq("") || trimmed.eq("/") || trimmed.eq("//") {
                        continue;
                    }

                    writeln!(merged_output_file, "{}*", &line)?;
                    writeln!(merged_output_file, "!{}", &line)?;
                }
            }
        }

        Ok(())
    })?;

    if let Err(_e) = perform("Applying the sparse profile", || {
        let merged_output_file =
            File::open(&merged_output_path).context("opening new sparse profile")?;
        let (mut cmd, scmd) = SandboxCommand::new_with_handles(
            git_helper::git_binary(),
            Some(Stdio::from(merged_output_file)),
            None,
            None,
            &sandbox,
        )?;
        scmd.ensure_success_or_log(
            cmd.current_dir(&repo)
                .arg("sparse-checkout")
                .arg("set")
                .arg("--stdin"),
            SandboxCommandOutput::Stderr,
            "sparse-checkout add",
        )
        .map(|_| ())
        .context("setting sparse checkout from new profile")
    }) {
        perform("Restoring and reapplying the backup profile", || {
            // std::fs::copy(&sparse_profile_path, &sparse_checkout_backup_path)
            //     .context("restoring the backup file")?;
            let backup_file = File::open(&sparse_checkout_backup_path)
                .context("opening backup sparse profile")?;
            let (mut cmd, scmd) = SandboxCommand::new_with_handles(
                git_helper::git_binary(),
                Some(Stdio::from(backup_file)),
                None,
                None,
                &sandbox,
            )?;
            scmd.ensure_success_or_log(
                cmd.current_dir(&repo)
                    .arg("sparse-checkout")
                    .arg("set")
                    .arg("--cone")
                    .arg("--stdin"),
                SandboxCommandOutput::Stderr,
                "sparse-checkout add",
            )
            .map(|_| ())
        })?;
    }

    Ok(())
}
