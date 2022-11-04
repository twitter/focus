// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

/*
This is a fast snapshotting mechanism that does not depend on `git stash`.
As of v2.38, `stash` requires full indices to work, so it is quite slow in
our repositories. We should revisit using this once `git stash` is made
fully sparse-index compatible.
*/

use anyhow::{bail, Context, Result};
use git2::Repository;
use lazy_static::lazy_static;
use std::{
    fs::File,
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tar::{Archive, Builder};

use crate::{app::App, git_helper, lock_file::LockFile};

use super::model::Disposition;

pub struct CreationResult {
    pub path: PathBuf,
}

lazy_static! {
    static ref TRACKED_CHANGE_PATCH_FILENAME: PathBuf = PathBuf::from("tracked-changes.patch");
}
/// When changes to the working tree are present, returns a snapshot containing the index and
/// changed files from the work tree. This archive has a simple structure and is meant to
/// replicate the working tree state by being extracted in the repo's top-level.
///
///
/// .git/index                     # The index file.
/// focus/tracked-changes.patch    # A patch of changes to tracked files.
/// a/changed/path                 # Untracked changed files ...
///
/// The archive should preserve ownership, modes, and extended attributes on the files.
pub fn create(repo_path: impl AsRef<Path>, app: Arc<App>) -> Result<Option<CreationResult>> {
    let app = app.clone();

    let repo_path = repo_path.as_ref();
    let git_dir = git_helper::git_dir(repo_path)?;
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Opening repo {} failed", repo_path.display()))?;

    let status = super::working_tree::status(repo_path, app.clone()).with_context(|| {
        format!(
            "Determining status of work tree {} failed",
            repo_path.display()
        )
    })?;

    // Check that there is anything to snapshot.
    if status.is_empty() {
        return Ok(None);
    }

    let sandbox = app.sandbox();
    // There are changes, let's get started...
    // Create a diff of tracked changes.
    let (tracked_change_patch_file, tracked_change_patch_path, _serial) =
        sandbox.create_file(Some("tracked-changes"), Some("patch"), None)?;
    let (mut cmd, scmd) = git_helper::git_command(app.clone())?;
    cmd.current_dir(repo_path.as_os_str())
        .arg("diff")
        .arg("HEAD")
        .stdout(Stdio::from(tracked_change_patch_file));
    scmd.ensure_success_or_log(
        &mut cmd,
        crate::sandbox_command::SandboxCommandOutput::Stderr,
    )?;

    // Create the archive file.
    let head_commit =
        git_helper::get_head_commit(&repo).context("Could not determine HEAD commit")?;
    let file_stem = hex::encode(head_commit.id());
    let (archive_file, archive_path, _serial) =
        sandbox.create_file(Some(file_stem.as_str()), Some("snapshot.tar"), None)?;

    let mut archive_builder = Builder::new(BufWriter::new(archive_file));

    // Add the index.
    {
        let index_path = git_dir.join("index");
        let index_lock_path = index_path.with_extension("lock");
        let _index_lock = LockFile::new(&index_lock_path).context("Locking the index failed")?;
        archive_builder
            .append_path_with_name(&index_path, ".git/index")
            .with_context(|| format!("Adding index from {}", index_path.display()))?;
    }

    // Add the patch of tracked changes.
    archive_builder
        .append_path_with_name(
            tracked_change_patch_path.as_path(),
            Path::new(".git").join(TRACKED_CHANGE_PATCH_FILENAME.as_path()),
        )
        .with_context(|| format!("Adding patch from {}", tracked_change_patch_path.display()))?;

    // Add untracked files.
    for entry in status
        .find_entries_with_disposition(Disposition::Untracked)
        .context("Failed to find untracked entries")?
    {
        let path = repo_path.join(&entry.path);

        archive_builder
            .append_path_with_name(&path, &entry.path)
            .with_context(|| format!("Failed to add file {}", path.display()))?;
    }

    // Clean up the working tree by cleaning untracked files.
    let _ = git_helper::run_consuming_stdout(repo_path, vec!["clean", "-f", "-d"], app.clone())?;
    let _ = git_helper::run_consuming_stdout(repo_path, vec!["reset", "--hard"], app.clone())?;

    Ok(Some(CreationResult { path: archive_path }))
}

pub fn apply(
    snapshot_path: impl AsRef<Path>,
    repo_path: impl AsRef<Path>,
    app: Arc<App>,
) -> Result<()> {
    let repo_path = repo_path.as_ref();
    let git_dir = git_helper::git_dir(repo_path)?;

    let status = super::working_tree::status(repo_path, app.clone()).with_context(|| {
        format!(
            "Determining status of work tree {} failed",
            repo_path.display()
        )
    })?;

    // Check that there is anything to snapshot.
    if !status.is_empty() {
        bail!("The working tree must be clean in order to restore a snapshot");
    }

    let snapshot_path = snapshot_path.as_ref();
    let mut snapshot_archive = Archive::new(BufReader::new(
        File::open(snapshot_path).with_context(|| {
            format!(
                "Failed to open snapshot archive {}",
                snapshot_path.display()
            )
        })?,
    ));

    snapshot_archive.set_overwrite(true);
    snapshot_archive.set_preserve_permissions(true);
    snapshot_archive.set_preserve_mtime(true);
    snapshot_archive.set_unpack_xattrs(true);

    snapshot_archive.unpack(repo_path).with_context(|| {
        format!(
            "Failed to unpack snapshot archive {}",
            snapshot_path.display()
        )
    })?;

    // Apply the patch
    let patch_path = git_dir.join(TRACKED_CHANGE_PATCH_FILENAME.as_path());
    let patch_path_str = patch_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Failed to convert patch path to a string"))?;

    let _ = git_helper::run_consuming_stdout(
        repo_path,
        vec!["apply", "-v", patch_path_str],
        app.clone(),
    )?;

    // Remove the patch file
    std::fs::remove_file(&patch_path)
        .with_context(|| format!("Failed to remove patch file {}", patch_path.display()))?;

    Ok(())
}

#[cfg(test)]
mod testing {
    use anyhow::Result;
    use focus_testing::{init_logging, ScratchGitRepo};

    use crate::git;

    use super::*;

    #[test]
    fn snapshot_smoke_test() -> Result<()> {
        init_logging();

        let app = Arc::new(App::new_for_testing()?);
        let repo_dir = app.sandbox().create_subdirectory("repo")?;

        let repo = ScratchGitRepo::new_static_fixture(&repo_dir)?;

        let tracked_removed_filename = PathBuf::from("a-tracked-file-added-and-later-deleted.txt");
        let tracked_removed_path = repo.path().join(&tracked_removed_filename);
        let tracked_removed_content = b"This file is added and later deleted.\n";
        std::fs::write(&tracked_removed_path, tracked_removed_content)?;
        repo.add_file(&tracked_removed_filename)?;
        repo.write_and_commit_file(
            &tracked_removed_filename,
            tracked_removed_content,
            format!("Commit of {}", tracked_removed_filename.display()),
        )?;
        repo.remove_file(&tracked_removed_filename)?;

        let tracked_filename = PathBuf::from("a-tracked-file.txt");
        let tracked_file_path = repo.path().join(&tracked_filename);
        let tracked_file_content = b"This file is added.\n";
        std::fs::write(&tracked_file_path, tracked_file_content)?;
        repo.add_file(&tracked_filename)?;

        let untracked_file_name = PathBuf::from("an-untracked-file.txt");
        let untracked_file_path = repo.path().join(&untracked_file_name);
        let untracked_content = b"This file is untracked.\n";
        std::fs::write(&untracked_file_path, untracked_content)?;

        // Check that the status is what we expect.
        let initial_status = git::working_tree::status(repo.path(), app.clone())?;
        {
            let untracked_entries =
                initial_status.find_entries_with_disposition(Disposition::Untracked)?;
            let untracked_entry = untracked_entries
                .iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("Expected an untracked entry and there was none"))?;
            assert_eq!(untracked_entry.path.as_path(), &untracked_file_name);

            let tracked_entries =
                initial_status.find_entries_with_disposition(Disposition::Added)?;
            {
                let tracked_entry = tracked_entries.iter().next().ok_or_else(|| {
                    anyhow::anyhow!("Expected an untracked entry and there was none")
                })?;
                assert_eq!(tracked_entry.path.as_path(), &tracked_filename);
            }
        }

        let snapshot = git::snapshot::create(repo.path(), app.clone())
            .context("Creating the snapshot failed")?
            .ok_or_else(|| anyhow::anyhow!("Expected a snapshot to be created"))?;
        let snapshot_stat =
            std::fs::metadata(&snapshot.path).context("Could not stat snapshot file")?;
        assert!(snapshot_stat.is_file());
        assert!(snapshot_stat.len() > 0);

        // After the snapshot is created, the tree should be in a clean state.
        {
            let status = git::working_tree::status(repo.path(), app.clone())?;
            tracing::debug!(entries = ?status.entries());
            assert!(status.is_empty());
        }

        // Apply the snapshot, check that everything is as before.
        {
            assert!(tracked_removed_path.is_file());

            git::snapshot::apply(&snapshot.path, repo.path(), app.clone())
                .context("Applying snapshot failed")?;

            // The patch should have removed the file.
            assert!(!tracked_removed_path.is_file());

            // The working tree status should be the same.
            let status = git::working_tree::status(repo.path(), app.clone())?;
            assert_eq!(status, initial_status);
        }

        Ok(())
    }
}
