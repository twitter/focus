// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{bail, Result};

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{app::App, git_helper};

use super::model::{Disposition, Kind};

/// Each entity represents a changed path in the working tree. Trees in the merge state are not supported.
#[derive(Debug, Hash, Eq, PartialEq)]
pub struct WorkingTreeStateEntry {
    pub kind: Kind,
    pub x: Disposition,
    pub y: Option<Disposition>,

    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
}

impl WorkingTreeStateEntry {
    // If there is a deleted path returns it.
    pub fn deleted_path(&self) -> Result<Option<PathBuf>> {
        match self.kind {
            Kind::Ordinary => {
                // If any char is 'D' there was a deletion
                if self.x == Disposition::Deleted || self.y == Some(Disposition::Deleted) {
                    Ok(Some(self.path.clone()))
                } else {
                    Ok(None)
                }
            }
            Kind::RenameOrCopy => {
                // Original path is removed. */
                Ok(Some(self.original_path.clone().ok_or_else(|| {
                    anyhow::anyhow!("Missing original path for rename/copy")
                })?))
            }
            _ => Ok(None),
        }
    }

    fn dispositions(&self) -> Vec<Disposition> {
        let mut dispositions = Vec::<Disposition>::new();
        dispositions.push(self.x);
        if let Some(y) = self.y {
            dispositions.push(y);
        }
        dispositions
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct WorkingTreeState {
    /// Entities organized by
    entries: Vec<WorkingTreeStateEntry>,

    /// Index of entity disposition to index in entities.
    by_disposition: HashMap<Disposition, Vec<usize>>,
}

impl WorkingTreeState {
    fn new() -> Self {
        let mut instance: WorkingTreeState = Default::default();
        for d in &[
            Disposition::Unmodified,
            Disposition::Modified,
            Disposition::FileTypeChanged,
            Disposition::Added,
            Disposition::Deleted,
            Disposition::Renamed,
            Disposition::Copied,
            Disposition::UpdatedButUnmerged,
            Disposition::Untracked,
            Disposition::Ignored,
        ] {
            instance.by_disposition.insert(*d, Vec::new());
        }
        instance
    }

    fn push(&mut self, entry: WorkingTreeStateEntry) -> Result<()> {
        let index = self.entries.len();

        // Add all dispositions to the index.
        for disposition in entry.dispositions() {
            let indices = self
                .by_disposition
                .get_mut(&disposition)
                .ok_or_else(|| anyhow::anyhow!("Missing disposition {:?}", disposition))?;
            indices.push(index);
        }

        self.entries.push(entry);

        Ok(())
    }

    /// Get a reference to the underlying entries.
    pub fn entries(&self) -> &Vec<WorkingTreeStateEntry> {
        &self.entries
    }

    /// Find entries with the given disposition
    pub fn find_entries_with_disposition(
        &self,
        disposition: Disposition,
    ) -> Result<Vec<&WorkingTreeStateEntry>> {
        let indices = self
            .by_disposition
            .get(&disposition)
            .ok_or_else(|| anyhow::anyhow!("Missing disposition {:?}", disposition))?;
        Ok(indices.iter().map(|i| &self.entries[*i]).collect())
    }

    /// Returns true if there are no changes to the working tree.
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Determine the line type from the initial token
fn determine_kind(token: &str) -> Option<Kind> {
    match token {
        s if s.eq("#") => Some(Kind::Header),
        s if s.eq("1") => Some(Kind::Ordinary),
        s if s.eq("2") => Some(Kind::RenameOrCopy),
        s if s.eq("u") => Some(Kind::Unmerged),
        s if s.eq("?") => Some(Kind::Untracked),
        s if s.eq("!") => Some(Kind::Untracked),
        _ => None,
    }
}

/// Parse a working tree status from the porcelain v2 format into a vector.
pub fn status(repo_path: impl AsRef<Path>, app: Arc<App>) -> Result<WorkingTreeState> {
    let mut state = WorkingTreeState::new();
    let output = git_helper::run_consuming_stdout(
        repo_path,
        ["status", "--porcelain=2", "-z", "--ignore-submodules=all"],
        app,
    )?;

    let mut null_delimited_split = output.split('\0');
    while let Some(frame) = null_delimited_split.next() {
        let tokens = frame.split_ascii_whitespace().collect::<Vec<&str>>();
        if tokens.is_empty() {
            break;
        }
        tracing::debug!(?tokens);
        if tokens.len() < 2 {
            bail!("Too few tokens in `{:?}`", tokens);
        }

        let mut token_iter = tokens.iter();
        let kind_token = token_iter
            .next()
            .ok_or_else(|| anyhow::anyhow!("Expected kind token (index 0)"))?;
        let kind = determine_kind(kind_token)
            .ok_or_else(|| anyhow::anyhow!("Unexpected initial token '{}'", kind_token))?;
        let path = tokens
            .last()
            .ok_or_else(|| anyhow::anyhow!("Missing path token"))?;

        // If the entry is a rename or copy it will have another frame with just the original path
        let original_path = if kind == Kind::RenameOrCopy {
            let frame = null_delimited_split.next().ok_or_else(|| {
                anyhow::anyhow!("Expected another frame containing the original path")
            })?;
            Some(PathBuf::from(frame))
        } else {
            None
        };

        // TODO: Expand the implementation here to parse all of the fields for each variant rather than taking only what we need.
        let flag_chars: Vec<char> = {
            // For untracked and ignored entries there is no initial numerical token, the first token is disposition
            let flags = match kind {
                Kind::Untracked => kind_token,
                Kind::Ignored => kind_token,
                _ => token_iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("Expected flag token (index 1)"))?,
            };
            flags.chars().collect()
        };
        if flag_chars.is_empty() {
            bail!("Disposition flags empty");
        }

        let x = Disposition::try_from(flag_chars[0])?;
        let y = if flag_chars.len() > 1 {
            Some(Disposition::try_from(flag_chars[1])?)
        } else {
            None
        };

        state.push(WorkingTreeStateEntry {
            kind,
            x,
            y,
            path: PathBuf::from(path),
            original_path,
        })?;
    }

    Ok(state)
}

#[cfg(test)]
mod testing {
    use anyhow::Result;
    use focus_testing::{init_logging, ScratchGitRepo};
    use tempfile::tempdir;

    use crate::git;

    use super::*;

    #[test]
    fn status_smoke_test() -> Result<()> {
        init_logging();

        let app = Arc::new(App::new_for_testing()?);
        let dir = tempdir().unwrap();
        let repo = ScratchGitRepo::new_static_fixture(dir.path())?;

        let file_name = PathBuf::from("file-1.txt");
        let untracked_file_path = repo.path().join(&file_name);
        std::fs::write(&untracked_file_path, b"Hello!\n")?;

        // Write a new file and check that its disposition is untracked
        {
            let status = git::working_tree::status(repo.path(), app.clone())?;
            let entries = status.find_entries_with_disposition(Disposition::Untracked)?;
            let entry = entries
                .first()
                .ok_or_else(|| anyhow::anyhow!("Expected an untracked entry and there was none"))?;
            assert_eq!(entry.path.as_path(), &file_name);
        }

        repo.add_file(&file_name)?;

        // After the file is added check that its disposition is staged
        {
            let status = git::working_tree::status(repo.path(), app.clone())?;
            let entries = status.find_entries_with_disposition(Disposition::Added)?;
            let entry = entries
                .first()
                .ok_or_else(|| anyhow::anyhow!("Expected an untracked entry and there was none"))?;
            assert_eq!(entry.path.as_path(), &file_name);
        }

        repo.commit_all("Add a file")?;

        // After we commit everything there are no changes to report
        {
            let status = git::working_tree::status(repo.path(), app.clone())?;
            assert!(status.is_empty());
        }

        drop(app);

        Ok(())
    }
}
