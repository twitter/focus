// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;

use std::{path::Path, sync::Arc};

use focus_util::{app::App, git_helper, lock_file::LockFile};

pub fn hold_lock(repo_path: &Path, file_name: &Path, app: Arc<App>) -> Result<LockFile> {
    let git_dir = git_helper::git_dir(repo_path, app.clone())?;
    let focus_dir = git_dir.join(".focus");
    std::fs::create_dir_all(&focus_dir)?;
    let lock_path = focus_dir.join(file_name);
    LockFile::new(&lock_path)
}
