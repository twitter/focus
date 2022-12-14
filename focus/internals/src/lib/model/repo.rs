// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use content_addressed_cache::RocksDBCache;
use focus_util::{
    app::App,
    git,
    git_helper::{self, get_head_commit, ConfigExt},
    paths::{self, is_build_definition},
    sandbox_command::SandboxCommandOutput,
};

use std::{
    collections::HashSet,
    fs,
    io::BufWriter,
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Duration,
};
use url::Url;

use crate::{
    hashing,
    index::{
        get_files_to_materialize, update_object_database_from_resolution, DependencyKey,
        HashContext, PathsToMaterializeResult,
    },
    model::outlining::{create_hierarchical_patterns, Pattern},
    project_cache::{ProjectCache, Value},
    target::TargetSet,
    target_resolver::{
        BazelResolutionStrategy, CacheOptions, ResolutionOptions, ResolutionRequest,
        ResolutionResult, Resolver, RoutingResolver,
    },
};

use super::{
    configuration::Configuration,
    outlining::{
        pattern_default_precedence, PatternContainer, PatternSet, PatternSetWriter,
        DEFAULT_OUTLINING_PATTERNS,
    },
    selection::{Selection, SelectionManager, Target},
};

use anyhow::{bail, Context, Result};
use git2::{ObjectType, Oid, Repository, TreeWalkMode, TreeWalkResult};
use tracing::{debug, info, info_span, trace, warn};
use uuid::Uuid;

const SPARSE_SYNC_REF_NAME: &str = "refs/focus/sync";
const PREEMPTIVE_SYNC_REF_NAME: &str = "refs/focus/presync";
const UUID_CONFIG_KEY: &str = "focus.uuid";
const PREEMPTIVE_SYNC_ENABLED_CONFIG_KEY: &str = "focus.preemptive-sync.enabled";
const PREEMPTIVE_SYNC_USER_IDLE_MILLIS_THRESHOLD_CONFIG_KEY: &str =
    "focus.preemptive-sync.user-idle-threshold";
const PREEMPTIVE_SYNC_USER_IDLE_MILLIS_THRESHOLD_DEFAULT: i32 = 15000;
const FILTER_VIEW: &str = "focus.filter";

const INDEX_SPARSE_CONFIG_KEY: &str = "index.sparse";
const CORE_UNTRACKED_CACHE_CONFIG_KEY: &str = "core.untrackedCache";

const OUTLINING_PATTERN_FILE_NAME: &str = "focus/outlining.patterns.json";
const LAST: usize = usize::MAX;

pub const PROJECT_CACHE_ENDPOINT_CONFIG_KEY: &str = "focus.project-cache.endpoint";
pub const PROJECT_CACHE_INCLUDE_HEADERS_FILE_CONFIG_KEY: &str =
    "focus.project-cache.include-headers-from";
pub const BAZEL_ONE_SHOT_RESOLUTION_CONFIG_KEY: &str = "focus.bazel.one-shot";

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WorkingTreeKind {
    Sparse,
    Dense,
}

/// Models a Git working tree.
pub struct WorkingTree {
    repo: git2::Repository,
}

impl std::fmt::Debug for WorkingTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { repo } = self;
        f.debug_struct("WorkingTree")
            .field("repo_path", &repo.path())
            .finish()
    }
}

impl PartialEq for WorkingTree {
    fn eq(&self, other: &Self) -> bool {
        let Self { repo } = self;
        let Self { repo: other_repo } = other;
        repo.path() == other_repo.path()
    }
}

impl Eq for WorkingTree {}

impl WorkingTree {
    /// Creates an instance.
    pub fn new(repo: git2::Repository) -> Result<Self> {
        if repo.workdir().is_none() {
            anyhow::bail!("Cannot create `WorkingTree` for bare repo");
        }
        Ok(Self { repo })
    }

    pub fn from_git_dir(git_dir: &Path) -> Result<Self> {
        let repo = git2::Repository::open(git_dir)
            .with_context(|| format!("Creating `WorkingTree` from git dir: {:?}", git_dir))?;
        Self::new(repo)
    }

    pub fn kind(&self) -> WorkingTreeKind {
        if self.sparse_checkout_path().is_file() {
            WorkingTreeKind::Sparse
        } else {
            WorkingTreeKind::Dense
        }
    }

    fn info_dir(&self) -> PathBuf {
        self.repo.path().join("info")
    }

    /// The location of the current sparse checkout file within the working tree.
    pub fn sparse_checkout_path(&self) -> PathBuf {
        self.info_dir().join("sparse-checkout")
    }

    /// Writes the given `patterns` to the working tree.
    pub fn apply_sparse_patterns(
        &self,
        patterns: PatternSet,
        cone: bool,
        app: Arc<App>,
    ) -> Result<bool> {
        // Make sure the patterns form a hierarchy
        let patterns = if cone {
            create_hierarchical_patterns(&patterns)
        } else {
            patterns
        };

        // Write the patterns
        let info_dir = self.info_dir();
        std::fs::create_dir_all(&info_dir)
            .with_context(|| format!("In working tree {}", self.work_dir().display()))
            .context("Failed to create leading directories for sparse profile")?;

        // Make sure Git doesn't try to do something cute with the sparse profile like merge existing contents.
        let sparse_profile_path = self.sparse_checkout_path();
        let candidate_sparse_profile_path = self
            .sparse_checkout_path()
            .with_extension(Path::new("candidate"));
        let new_content_hash = patterns.write_to_file(&candidate_sparse_profile_path)?;

        if sparse_profile_path.is_file() {
            let existing_content_hash = hashing::hash_file(&sparse_profile_path)
                .context("Hashing contents of existing sparse profile failed")?;
            if !existing_content_hash.is_empty() && existing_content_hash == new_content_hash {
                // We wrote the exact same thing. Skip everything.
                info!(profile = ?sparse_profile_path, "Skipping application of the sparse profile because it has not changed");
                std::fs::remove_file(&candidate_sparse_profile_path)
                    .context("Removing candidate sparse profile")?;
                return Ok(false);
            } else {
                info!(profile = ?sparse_profile_path, "Sparse profile changed");
            }
        }
        std::fs::rename(&candidate_sparse_profile_path, &sparse_profile_path)
            .context("Moving candidate sparse profile into place")?;

        // Update the working tree to match
        info!(profile = ?sparse_profile_path, count = %patterns.len(), "Applying patterns");
        {
            let args = vec![
                "sparse-checkout",
                "init",
                if cone { "--cone" } else { "--no-cone" },
            ];
            let (mut cmd, scmd) = git_helper::git_command(app.clone())?;
            scmd.ensure_success_or_log(
                cmd.current_dir(self.work_dir()).args(args),
                SandboxCommandOutput::Stderr,
            )
            .with_context(|| format!("In working tree {}", self.work_dir().display()))
            .context("git sparse-checkout init failed")?;
        }

        // Newer versions of Git don't actually check out files when `sparse-checkout init` runs, so run `git checkout`. It might be worth making this behavior version-dependent.
        info!("Checking out");
        {
            let args = vec!["checkout"];
            let (mut cmd, scmd) = git_helper::git_command(app)?;
            scmd.ensure_success_or_log(
                cmd.current_dir(self.work_dir()).args(args),
                SandboxCommandOutput::Stderr,
            )
            .with_context(|| format!("In working tree {:?}", self.work_dir()))
            .context("git checkout failed")?;
        }

        Ok(true)
    }

    /// Switch to the given commit in this working tree.
    pub fn switch_to_commit(
        &self,
        commit_id: git2::Oid,
        detach: bool,
        discard_changes: bool,
        app: Arc<App>,
    ) -> Result<()> {
        // Update the working tree to match
        let commit_id_str = commit_id.to_string();
        let span = info_span!("Switching to detached commit", commit_id = %commit_id_str, path = ?self.work_dir());
        let _guard = span.enter();

        let mut args = vec!["switch", "--ignore-other-worktrees"];
        if detach {
            args.push("--detach");
        }
        if discard_changes {
            args.push("--discard-changes");
        }
        args.push(&commit_id_str);

        let (mut cmd, scmd) = git_helper::git_command(app)?;
        scmd.ensure_success_or_log(
            cmd.current_dir(self.work_dir()).args(args),
            SandboxCommandOutput::Stderr,
        )
        .with_context(|| {
            format!(
                "Switching to commit {} failed (detach={}, discard_changes={})",
                &commit_id_str, detach, discard_changes
            )
        })?;

        Ok(())
    }

    /// Get a reference to the working tree's path.
    pub fn work_dir(&self) -> &Path {
        // Verified by constructor.
        self.repo.workdir().unwrap()
    }

    /// Get a reference to the working tree's git dir.
    pub fn git_dir(&self) -> &Path {
        self.repo.path()
    }

    pub fn default_working_tree_patterns(&self) -> Result<PatternSet> {
        let mut patterns = PatternSet::new();
        patterns.insert(Pattern::Directory {
            precedence: patterns.len(),
            path: PathBuf::default(),
            recursive: true,
        });
        patterns.insert(Pattern::Directory {
            precedence: pattern_default_precedence(),
            path: PathBuf::from("focus"),
            recursive: true,
        });
        Ok(patterns)
    }

    #[allow(dead_code)]
    fn locate_paths_in_tree(&self, prefixes: &HashSet<PathBuf>) -> Result<PatternSet> {
        let mut results = PatternSet::new();
        let head_commit = self.get_head_commit()?;
        let head_tree = head_commit.tree().context("Getting HEAD tree")?;
        head_tree
            .walk(TreeWalkMode::PreOrder, |s, _| {
                if s.is_empty() {
                    return TreeWalkResult::Ok;
                }
                let path = PathBuf::from(s);
                let first_component = path
                    .components()
                    .take(1)
                    .next()
                    .map(|c| -> PathBuf { PathBuf::from(c.as_os_str()) })
                    .unwrap();

                if prefixes.contains(&first_component) {
                    results.insert(Pattern::Directory {
                        precedence: results.len(),
                        path,
                        recursive: true,
                    });
                    TreeWalkResult::Ok
                } else {
                    TreeWalkResult::Skip
                }
            })
            .context("Failed to walk tree")?;

        Ok(results)
    }

    fn apply_working_tree_patterns(&self, app: Arc<App>) -> Result<bool> {
        let patterns = self.default_working_tree_patterns()?;
        self.apply_sparse_patterns(patterns, true, app)
            .context("Failed to apply root-only patterns")
    }

    pub fn read_ref(&self, name: &str) -> Result<Option<Oid>> {
        let reference = self.repo.find_reference(name).with_context(|| {
            format!(
                "Finding sync reference {} in repo {}",
                name,
                self.repo.path().display()
            )
        });
        match reference {
            Ok(reference) => {
                let commit = reference.peel_to_commit().with_context(|| {
                    format!(
                        "Resolving commit for reference {} in repo {}",
                        name,
                        self.repo.path().display()
                    )
                })?;
                Ok(Some(commit.id()))
            }
            _ => Ok(None),
        }
    }

    /// Reads the commit ID of the sparse sync ref (named SYNC_REF_NAME)
    pub fn read_sparse_sync_point_ref(&self) -> Result<Option<Oid>> {
        self.read_ref(SPARSE_SYNC_REF_NAME)
    }

    /// Reads the commit ID of the preemptive sync ref (named SYNC_REF_NAME)
    pub fn read_preemptive_sync_point_ref(&self) -> Result<Option<Oid>> {
        self.read_ref(PREEMPTIVE_SYNC_REF_NAME)
    }

    pub fn primary_branch_name(&self) -> Result<String> {
        if self.repo.find_reference("refs/heads/master").is_ok() {
            Ok(String::from("master"))
        } else if self.repo.find_reference("refs/heads/main").is_ok() {
            Ok(String::from("main"))
        } else {
            bail!("Could not determine primary branch name")
        }
    }

    pub fn write_sync_point_ref_internal(&self, name: &str, commit_id: git2::Oid) -> Result<()> {
        self.repo
            .reference(SPARSE_SYNC_REF_NAME, commit_id, true, "focus sync")
            .with_context(|| {
                format!(
                    "Recording sync point ref {} in repo {} to {}",
                    name,
                    self.work_dir().display(),
                    &commit_id,
                )
            })
            .map(|_| ())
    }

    /// Updates the sparse sync ref to the value of the HEAD ref (named SYNC_REF_NAME)
    pub fn write_sync_point_ref(&self) -> Result<()> {
        let head_commit = self
            .get_head_commit()
            .context("Determining the HEAD commit")?;
        self.write_sync_point_ref_internal(SPARSE_SYNC_REF_NAME, head_commit.id())
            .context("Updating the sparse sync ref")
    }

    /// Updates the sparse sync ref to the value of the HEAD ref (named SYNC_REF_NAME)
    pub fn write_preemptive_sync_point_ref(&self, commit_id: git2::Oid) -> Result<()> {
        self.write_sync_point_ref_internal(PREEMPTIVE_SYNC_REF_NAME, commit_id)
            .context("Updating the preemptive sync ref")
    }

    pub fn git_repo(&self) -> &Repository {
        &self.repo
    }

    /// Determine if the working tree is clean
    pub fn is_clean(&self, app: Arc<App>) -> Result<bool> {
        Ok(
            git_helper::run_consuming_stdout(self.work_dir(), vec!["status", "--porcelain"], app)?
                .trim()
                .is_empty(),
        )
    }

    pub fn read_uuid(&self) -> Result<Option<Uuid>> {
        let config_snapshot = self.repo.config()?.snapshot()?;
        match config_snapshot.get_str(UUID_CONFIG_KEY) {
            Ok(uuid) => {
                let uuid = Uuid::from_str(uuid)?;
                Ok(Some(uuid))
            }
            Err(_) => Ok(None),
        }
    }

    pub fn write_generated_uuid(&self) -> Result<Uuid> {
        let uuid = Uuid::new_v4();
        self.repo
            .config()?
            .set_str(UUID_CONFIG_KEY, uuid.to_string().as_str())?;
        Ok(uuid)
    }

    pub fn get_head_commit(&self) -> Result<git2::Commit> {
        get_head_commit(&self.repo)
    }

    pub fn configure(&self, app: Arc<App>) -> Result<()> {
        if self.kind() == WorkingTreeKind::Dense {
            // We do not perform this configuration in dense repos.
            return Ok(());
        }

        let config_snapshot = self.repo.config()?.snapshot()?;

        if config_snapshot.get_str(INDEX_SPARSE_CONFIG_KEY).is_err() {
            git_helper::write_config(self.git_dir(), INDEX_SPARSE_CONFIG_KEY, "true", app.clone())
                .context("Configuring sparse index")?;
        }
        if config_snapshot
            .get_str(CORE_UNTRACKED_CACHE_CONFIG_KEY)
            .is_err()
        {
            git_helper::write_config(self.git_dir(), CORE_UNTRACKED_CACHE_CONFIG_KEY, "true", app)
                .context("Configuring untracked cache")?;
        }

        Ok(())
    }

    pub fn set_filter_config(&self, val: bool) -> Result<()> {
        self.repo
            .config()?
            .set_str(FILTER_VIEW, val.to_string().as_str())?;
        Ok(())
    }

    pub fn get_filter_config(&self) -> Result<bool> {
        let config_snapshot = self.repo.config()?.snapshot()?;
        Ok(config_snapshot.get_bool(FILTER_VIEW).unwrap_or(true))
    }

    pub fn switch_filter_off(&self, app: Arc<App>) -> Result<()> {
        //save of the current copy of .git/info/sparse-checkout as git/info/sparse-checkout.filtered
        let sparse_profile_path = self.sparse_checkout_path();
        if sparse_profile_path.is_file() {
            let filtered_sparse_profile_path = self
                .sparse_checkout_path()
                .with_extension(Path::new("filtered"));

            std::fs::copy(&sparse_profile_path, &filtered_sparse_profile_path)
                .context("Copying sparse profile to .filtered")?;

            info!(profile = ?sparse_profile_path, "Backed up sparse profile.");
        }

        info!("Updating sparse profile");
        let unfiltered_sparse_profile_path = self
            .sparse_checkout_path()
            .with_extension(Path::new("dense"));
        std::fs::write(&unfiltered_sparse_profile_path, "/*\n")
            .context("Writing dense sparse profile")?;
        std::fs::rename(&unfiltered_sparse_profile_path, &sparse_profile_path)
            .context("Moving unfiltered sparse profile into place")?;

        info!("Updating worktree...");
        self.filter_update_worktree(app)?;

        Ok(())
    }

    pub fn switch_filter_on(&self, app: Arc<App>) -> Result<()> {
        let sparse_profile_path = self.sparse_checkout_path();
        let filtered_sparse_profile_path = self
            .sparse_checkout_path()
            .with_extension(Path::new("filtered"));

        if !filtered_sparse_profile_path.is_file() {
            // in this case will need to rely on a sync run to reinstate a filtered sparse profile
            info!("No filtered sparse file to reinstate. Will need a sync to update worktree.");
            return Ok(());
        }

        info!("Updating sparse profile");
        std::fs::rename(&filtered_sparse_profile_path, &sparse_profile_path)
            .context("Moving filtered sparse profile into place")?;

        info!("Updating worktree...");
        self.filter_update_worktree(app)?;

        Ok(())
    }

    fn filter_update_worktree(&self, app: Arc<App>) -> Result<()> {
        let args = vec!["sparse-checkout", "reapply"];
        let (mut cmd, scmd) = git_helper::git_command(app)?;
        scmd.ensure_success_or_log(
            cmd.current_dir(self.work_dir()).args(args),
            SandboxCommandOutput::Stderr,
        )
        .with_context(|| format!("In working tree {}", self.work_dir().display()))
        .context("git sparse-checkout reapply failed")?;

        Ok(())
    }
}

pub trait Outliner {
    /// Get the patterns associated with the provided targets at a given revision.
    fn outline(
        &self,
        commit_id: git2::Oid,
        target_set: &TargetSet,
        resolution_options: &ResolutionOptions,
        snapshot: Option<PathBuf>,
        app: Arc<App>,
    ) -> Result<(PatternSet, ResolutionResult)>;

    fn underlying(&self) -> Arc<WorkingTree>;

    fn identity(&self) -> &str;
}

/// A specialization of a WorkingTree used for outlining tasks, containing only files related to, and necessary for querying, the build graph.
#[derive(Debug, PartialEq, Eq)]
pub struct OutliningTreeOutliner {
    underlying: Arc<WorkingTree>,
}

impl OutliningTreeOutliner {
    pub fn new(underlying: Arc<WorkingTree>) -> Self {
        Self { underlying }
    }

    pub fn underlying(&self) -> Arc<WorkingTree> {
        self.underlying.clone()
    }

    fn apply_configured_outlining_patterns(
        &self,
        commit_id: git2::Oid,
        app: Arc<App>,
    ) -> Result<bool> {
        let patterns = self.configured_outlining_patterns(commit_id)?;
        self.underlying
            .apply_sparse_patterns(patterns, false, app)
            .context("Failed to apply build file patterns")
    }

    /// Read configured outlining patterns from the repository at the given commit.
    fn configured_outlining_patterns(&self, commit_id: git2::Oid) -> Result<PatternSet> {
        let repo = self.underlying();
        let repo = repo.git_repo();
        let commit = repo.find_commit(commit_id).context("Resolving commit")?;
        let tree = commit.tree().context("Resolving tree")?;
        let pattern_file = match tree.get_path(Path::new(OUTLINING_PATTERN_FILE_NAME)) {
            Ok(pattern_file) => pattern_file,
            Err(err) if err.code() == git2::ErrorCode::NotFound => {
                return Ok(DEFAULT_OUTLINING_PATTERNS.clone())
            }
            Err(err) => {
                anyhow::bail!(
                    "The outlining pattern file (named '{}') could not be loaded from the repository at this commit ({}): {}",
                    OUTLINING_PATTERN_FILE_NAME,
                    &commit.id().to_string(),
                    err,
                );
            }
        };
        let pattern_object = pattern_file.to_object(repo).context("Resolving object")?;
        let pattern_blob = pattern_object.as_blob().ok_or_else(|| {
            anyhow::anyhow!(
                "Expected {} to be a blob, but it was not",
                OUTLINING_PATTERN_FILE_NAME
            )
        })?;

        let pattern_container: PatternContainer = serde_json::from_slice(pattern_blob.content())
            .with_context(|| {
                format!(
                    "Parsing outline pattern file '{}' (at commit {})",
                    OUTLINING_PATTERN_FILE_NAME,
                    &commit.id().to_string()
                )
            })?;

        Ok(pattern_container.patterns)
    }
}

impl Outliner for OutliningTreeOutliner {
    fn outline(
        &self,
        commit_id: git2::Oid,
        target_set: &TargetSet,
        resolution_options: &ResolutionOptions,
        snapshot: Option<PathBuf>,
        app: Arc<App>,
    ) -> Result<(PatternSet, ResolutionResult)> {
        let repo = self.underlying();
        let git_repo = repo.git_repo();
        self.apply_configured_outlining_patterns(commit_id, app.clone())
            .context("Applying configured outlining patterns failed")?;
        self.underlying()
            .switch_to_commit(commit_id, true, true, app.clone())
            .context("Failed to switch to commit")?;
        if let Some(snapshot_path) = snapshot {
            let repo_workdir = git_repo
                .workdir()
                .ok_or_else(|| anyhow::anyhow!("Repository has no workdir"))?;
            git::snapshot::apply(snapshot_path, repo_workdir, false, app.clone())
                .context("Applying patch to outlining tree failed")?;
        }
        outline_common(git_repo, target_set, resolution_options, app, commit_id)
    }

    fn underlying(&self) -> Arc<WorkingTree> {
        self.underlying.clone()
    }

    fn identity(&self) -> &str {
        "OutliningTreeOutliner"
    }
}

/// The dense repo outliner performs outlining directly in a dense working tree.
#[derive(Debug, PartialEq, Eq)]
pub struct DenseRepoOutliner {
    underlying: Arc<WorkingTree>,
}

impl DenseRepoOutliner {
    pub fn new(underlying: Arc<WorkingTree>) -> Self {
        Self { underlying }
    }
}

impl Outliner for DenseRepoOutliner {
    /// Outline in the dense repo. Cannot switch commits. Never applies snapshots, complains if they are passed.
    fn outline(
        &self,
        commit_id: git2::Oid,
        target_set: &TargetSet,
        resolution_options: &ResolutionOptions,
        snapshot: Option<PathBuf>,
        app: Arc<App>,
    ) -> Result<(PatternSet, ResolutionResult)> {
        let underlying_repo = self.underlying();
        let checked_out_commit = underlying_repo.get_head_commit()?;
        let checked_out_commit_id = checked_out_commit.id();
        if checked_out_commit_id != commit_id {
            bail!(
                "Dense tree is at commit {} rather than the expected commit {}",
                hex::encode(checked_out_commit.id().as_bytes()),
                hex::encode(commit_id.as_bytes())
            )
        }

        if snapshot.is_some() {
            bail!("Cannot outline in a dense repo with changes present");
        }

        outline_common(
            self.underlying().git_repo(),
            target_set,
            resolution_options,
            app,
            commit_id,
        )
    }

    fn underlying(&self) -> Arc<WorkingTree> {
        self.underlying.clone()
    }

    fn identity(&self) -> &str {
        "DenseRepoOutliner"
    }
}

impl DenseRepoOutliner {}

fn make_routing_resolver() -> Result<RoutingResolver> {
    let cache_dir = dirs::cache_dir()
        .context("failed to determine cache dir")?
        .join("focus")
        .join("cache");
    Ok(RoutingResolver::new(cache_dir.as_path()))
}

fn outline_common(
    repository: &Repository,
    target_set: &HashSet<Target>,
    resolution_options: &ResolutionOptions,
    app: Arc<App>,
    commit_id: Oid,
) -> Result<(PatternSet, ResolutionResult), anyhow::Error> {
    let repo_workdir = repository
        .workdir()
        .ok_or_else(|| anyhow::anyhow!("Repository has no workdir"))?;
    let cache_options = CacheOptions::default();
    let request = ResolutionRequest {
        repo: repo_workdir.to_owned(),
        targets: target_set.clone(),
        options: resolution_options.clone(),
    };
    let mut patterns = PatternSet::new();
    let resolver = make_routing_resolver()?;
    let result = resolver.resolve(&request, &cache_options, app)?;
    for path in result.paths.iter() {
        let qualified_path = repo_workdir.join(path);

        let path = find_closest_directory_with_build_file(repository, commit_id, &qualified_path)
            .context("Failed locating closest build file")?
            .unwrap_or(qualified_path);
        if let Some(path) = treat_path(repo_workdir, &path)? {
            patterns.insert(Pattern::Directory {
                precedence: LAST,
                path,
                recursive: true,
            });
        }
    }
    Ok((patterns, result))
}

fn treat_path(repo_path: impl AsRef<Path>, path: impl AsRef<Path>) -> Result<Option<PathBuf>> {
    let repo_path = repo_path.as_ref();
    let p = path.as_ref();
    let p = p
        .strip_prefix(repo_path)
        .context("Failed to strip repo path prefix")?;

    if p == paths::MAIN_SEPARATOR_PATH.as_path() {
        Ok(None)
    } else {
        Ok(Some(p.to_owned()))
    }
}

fn find_closest_directory_with_build_file(
    git_repo: &git2::Repository,
    commit_id: git2::Oid,
    path: impl AsRef<Path>,
    // ceiling: impl AsRef<Path>,
) -> Result<Option<PathBuf>> {
    let path = path.as_ref();
    let tree = git_repo
        .find_commit(commit_id)
        .context("Resolving commit")?
        .tree()
        .context("Resolving tree")?;

    let mut path = path.to_owned();
    loop {
        if let Ok(tree_entry) = tree.get_path(&path) {
            // If the entry is a tree, get it.
            if tree_entry.kind() == Some(ObjectType::Tree) {
                let tree_object = tree_entry
                    .to_object(git_repo)
                    .with_context(|| format!("Resolving tree {}", path.display()))?;
                let current_tree = tree_object.as_tree().unwrap();
                // Iterate through the tree to see if there is a build file.
                for entry in current_tree.iter() {
                    if entry.kind() == Some(ObjectType::Blob) {
                        if let Some(name) = entry.name() {
                            let candidate_path = PathBuf::from_str(name)?;
                            if is_build_definition(candidate_path) {
                                info!(?name, ?path, "Found build definition");

                                return Ok(Some(path));
                            }
                        }
                    }
                }
            }
        }

        if !path.pop() {
            // We have reached the root with no match.
            break;
        }
    }

    Ok(None)
}

const OUTLINING_TREE_NAME: &str = "outlining-tree";

pub struct Repo {
    path: PathBuf,
    git_dir: PathBuf,
    working_tree: Option<Arc<WorkingTree>>,
    outliner: Option<Arc<dyn Outliner>>,
    repo: git2::Repository,
    config: Configuration,
    app: Arc<App>,
}

impl Repo {
    pub fn open(path: &Path, app: Arc<App>) -> Result<Self> {
        let repo = git2::Repository::open(path)
            .with_context(|| format!("Opening repo {}", path.display()))
            .context("Failed to open repo")?;
        if repo.is_bare() {
            bail!("Bare repos are not supported");
        }
        let git_dir = repo.path().to_owned();
        let working_tree: Option<Arc<WorkingTree>> = match repo.workdir() {
            Some(work_dir) => {
                let repo = git2::Repository::open(work_dir)?;
                Some(Arc::new(WorkingTree::new(repo)?))
            }
            None => None,
        };

        let outlining_tree_path = Self::outlining_tree_path(&git_dir);
        let outlining_tree_git_dir = git_dir.join("worktrees").join(OUTLINING_TREE_NAME);
        let outlining_tree: Option<Arc<dyn Outliner>> = if outlining_tree_path.is_dir() {
            Some(Arc::new(OutliningTreeOutliner::new(Arc::new(
                WorkingTree::from_git_dir(&outlining_tree_git_dir)?,
            ))))
        } else {
            match working_tree.as_ref() {
                Some(working_tree) if working_tree.kind() == WorkingTreeKind::Dense => {
                    Some(Arc::new(DenseRepoOutliner::new(working_tree.clone())))
                }
                _ => None,
            }
        };

        let config = Configuration::new(path).context("Loading configuration")?;
        let path = path.to_owned();

        Ok(Self {
            path,
            git_dir,
            working_tree,
            outliner: outlining_tree,
            repo,
            config,
            app,
        })
    }

    pub fn underlying(&self) -> &git2::Repository {
        &self.repo
    }

    pub fn config(&self) -> &Configuration {
        &self.config
    }

    pub fn focus_git_dir_path(git_dir: &Path) -> PathBuf {
        git_dir.join("focus")
    }

    pub fn outlining_tree_path(git_dir: &Path) -> PathBuf {
        Self::focus_git_dir_path(git_dir).join(OUTLINING_TREE_NAME)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run a sync, returning the number of patterns that were applied and whether a checkout occured as a result of the profile changing.
    pub fn sync(
        &self,
        commit_id: git2::Oid,
        targets: &TargetSet,
        skip_pattern_application: bool,
        app: Arc<App>,
        cache: Option<&RocksDBCache>,
        snapshot: Option<PathBuf>,
    ) -> Result<(usize, bool)> {
        let (working_tree, outlining_tree) = match (&self.working_tree, &self.outliner) {
            (Some(working_tree), Some(outlining_tree)) => (working_tree, outlining_tree),
            _ => {
                // TODO: we might succeed in synchronization without an outlining tree.
                bail!("Synchronization is only possible in a repo with both working and outlining trees");
            }
        };

        // Ensure that the trees are properly configured
        working_tree
            .configure(app.clone())
            .context("Configuring the working tree")?;
        outlining_tree
            .underlying()
            .configure(app.clone())
            .context("Configuring the outlining tree")?;

        let mut outline_patterns = if let Some(cache) = cache {
            self.sync_incremental(
                commit_id,
                targets,
                outlining_tree.as_ref(),
                cache,
                snapshot,
                app.clone(),
            )
        } else {
            self.sync_one_shot(
                commit_id,
                targets,
                outlining_tree.as_ref(),
                snapshot,
                app.clone(),
            )
        }?;

        outline_patterns.extend(working_tree.default_working_tree_patterns()?);
        let pattern_count = outline_patterns.len();
        let checked_out = if skip_pattern_application {
            false
        } else {
            working_tree
                .apply_sparse_patterns(outline_patterns, true, app)
                .context("Failed to apply outlined patterns to working tree")?
        };

        Ok((pattern_count, checked_out))
    }

    /// Sync in one shot, not using the cache.
    fn sync_one_shot(
        &self,
        commit_id: Oid,
        targets: &HashSet<Target>,
        outliner: &dyn Outliner,
        snapshot: Option<PathBuf>,
        app: Arc<App>,
    ) -> Result<PatternSet> {
        info!("Running one-shot sync");
        let resolution_options = ResolutionOptions {
            bazel_resolution_strategy: BazelResolutionStrategy::OneShot,
        };
        let (outline_patterns, _resolution_result) = outliner
            .outline(commit_id, targets, &resolution_options, snapshot, app)
            .context("Failed to outline")?;
        Ok(outline_patterns)
    }

    /// Sync using the cache, outlining when necessary recursively on dependencies.
    fn sync_incremental(
        &self,
        commit_id: Oid,
        targets: &HashSet<Target>,
        outliner: &dyn Outliner,
        cache: &RocksDBCache,
        snapshot: Option<PathBuf>,
        app: Arc<App>,
    ) -> Result<PatternSet> {
        let index_config = &self.config().index;
        let commit = self
            .underlying()
            .find_commit(commit_id)
            .with_context(|| format!("Resolving commit {}", commit_id))?;
        let tree = commit.tree().context("Resolving tree")?;
        let hash_context = HashContext::new(&self.repo, &tree)?;
        let ti_client = app.tool_insights_client();
        let dependency_keys: HashSet<DependencyKey> =
            targets.iter().cloned().map(DependencyKey::from).collect();
        info!("Checking cache for sparse checkout patterns");
        let mut paths_to_materialize =
            get_files_to_materialize(&hash_context, cache, dependency_keys.clone())?;
        if index_config.enabled {
            if let PathsToMaterializeResult::MissingKeys { .. } = paths_to_materialize {
                info!("Cache miss for sparse checkout patterns; fetching from the remote index");
                // TODO: Re-enable after the index is moved into its own crate.
                // let _: Result<ExitCode> = index::fetch_internal(
                //     app.clone(),
                //     cache,
                //     working_tree.work_dir().to_path_buf(),
                //     index_config,
                // );
                // Query again now that the index is populated.
                paths_to_materialize =
                    get_files_to_materialize(&hash_context, cache, dependency_keys)?;
            }
        }
        Ok(match paths_to_materialize {
            PathsToMaterializeResult::Ok { seen_keys, paths } => {
                info!(
                    num_seen_keys = seen_keys.len(),
                    "Cache hit for sparse checkout patterns"
                );
                ti_client
                    .get_context()
                    .add_to_custom_map("index_miss_count", "0");
                ti_client
                    .get_context()
                    .add_to_custom_map("index_hit_count", seen_keys.len().to_string());
                paths
                    .into_iter()
                    .map(|path| Pattern::Directory {
                        precedence: LAST,
                        path,
                        recursive: true,
                    })
                    .collect()
            }

            PathsToMaterializeResult::MissingKeys {
                seen_keys,
                missing_keys,
            } => {
                info!(
                    num_missing_keys = ?missing_keys.len(),
                    "Cache miss for sparse checkout patterns; querying Bazel"
                );

                // Write a file listing missing DependencyKeys.
                {
                    let (file, _, _) =
                        self.app
                            .sandbox()
                            .create_file(Some("missing-keys"), Some("txt"), None)?;
                    let mut writer = BufWriter::new(file);
                    for (dependency_key, _) in missing_keys.iter() {
                        writeln!(&mut writer, "{:?}", dependency_key)?;
                    }
                }

                ti_client
                    .get_context()
                    .add_to_custom_map("index_miss_count", missing_keys.len().to_string());
                ti_client
                    .get_context()
                    .add_to_custom_map("index_hit_count", seen_keys.len().to_string());

                debug!(?missing_keys, "These are the missing keys");
                let resolution_options = ResolutionOptions {
                    bazel_resolution_strategy: BazelResolutionStrategy::Incremental,
                };
                let (outline_patterns, resolution_result) = outliner
                    .outline(
                        commit_id,
                        targets,
                        &resolution_options,
                        snapshot,
                        app.clone(),
                    )
                    .context("Failed to outline")?;

                debug!(?resolution_result, ?outline_patterns, "Resolved patterns");
                update_object_database_from_resolution(&hash_context, cache, &resolution_result)?;
                outline_patterns
            }
        })
    }

    /// Sync using the project cache returning an optional value of the number of patterns and whether a checkout occured. None is returned if the project cache could not be used.
    pub fn sync_using_project_cache(
        &self,
        commit_id: git2::Oid,
        selection: &Selection,
        snapshot: Option<PathBuf>,
    ) -> Result<Option<(usize, bool)>> {
        if !selection.targets.is_empty() {
            tracing::warn!("Skipping project cache because the selection contains ad-hoc targets");
            return Ok(None);
        }

        let project_names: Vec<&String> = selection
            .projects
            .iter()
            .filter(|project| project.is_selectable())
            .map(|project| &project.name)
            .collect();
        debug!(?selection, ?project_names, "Selection");

        let endpoint = self.get_project_cache_remote_endpoint()?;
        if endpoint.is_none() {
            tracing::warn!(
                "Skipping project cache because no remote is configured (set {} to configure one)",
                PROJECT_CACHE_ENDPOINT_CONFIG_KEY
            );
            return Ok(None);
        }
        let endpoint = endpoint.unwrap();
        let endpoint_str = endpoint.as_str().to_owned();
        let cache = ProjectCache::new(self, endpoint, self.app.clone())?;

        // TODO: Prefetch build graph hash data
        let (_, build_graph_hash) = cache.get_build_graph_hash(commit_id, true)?;

        // Check whether we have a build graph hash locally to determine whether we've already fetched
        let need_fetch = !cache.is_imported(&build_graph_hash).context(
            "Failed to determine whether the necessary project cache data has been imported",
        )?;

        // Actually calculate the graph hash if we dont' have it so that we can actually pull
        let build_graph_hash_str = hex::encode(&build_graph_hash);

        if need_fetch {
            tracing::info!(build_graph_hash = ?build_graph_hash_str, endpoint = &endpoint_str, "Fetching content from remote project cache");
            cache
                .fetch(&build_graph_hash)
                .context("Fetching content failed")?;
        }

        let working_tree = self.working_tree()?;
        let mut outline_patterns = working_tree.default_working_tree_patterns()?;
        let mut missing_projects = Vec::<&String>::new();

        // Add mandatory project patterns
        {
            let (_key, mandatory_project_patterns) = cache.get_mandatory_project_patterns(
                commit_id,
                &build_graph_hash,
                false,
                snapshot.clone(),
            )?;
            let mandatory_project_patterns = mandatory_project_patterns
                .ok_or_else(|| anyhow::anyhow!("Missing mandatory project patterns"))?;
            match mandatory_project_patterns {
                Value::MandatoryProjectPatternSet(patterns) => {
                    tracing::info!(count = patterns.len(), "Adding mandatory patterns");
                    outline_patterns.extend(patterns)
                }
                _ => unreachable!("Unexpected value type"),
            };
        }

        // Add optional project patterns
        for project_name in project_names {
            match cache.get_optional_project_patterns(
                commit_id,
                &build_graph_hash,
                project_name,
                false,
                &PatternSet::new(),
                snapshot.clone(),
            )? {
                (_key, Some(Value::OptionalProjectPatternSet(patterns))) => {
                    outline_patterns.extend(patterns);
                }
                (key, Some(val)) => {
                    bail!("Unexpected value ({:?}) for key {:?}, expected an ProjectCacheValue::OptionalProjectPatternSet", val, key);
                }
                (_key, None) => {
                    missing_projects.push(project_name);
                }
            }
        }

        if !missing_projects.is_empty() {
            tracing::warn!(
                ?missing_projects,
                "Project cache cannot be used since it is missing content"
            );
            return Ok(None);
        }

        // Ensure that the working tree is properly configured
        working_tree
            .configure(self.app.clone())
            .context("Configuring the working tree")?;
        trace!(?outline_patterns);

        // TODO: Implement skipping application if the profile has not changed
        let pattern_count = outline_patterns.len();
        let checked_out = working_tree
            .apply_sparse_patterns(outline_patterns, true, self.app.clone())
            .context("Failed to apply outlined patterns to working tree")?;
        info!("Synced from project cache");
        Ok(Some((pattern_count, checked_out)))
    }

    /// Creates an outlining tree for the repository.
    pub fn create_outlining_tree(&self) -> Result<()> {
        let path = Self::outlining_tree_path(&self.git_dir);
        if path.is_dir() {
            bail!("Refusing to create outlining tree since the directory already exists.")
        }

        fs::create_dir_all(Self::focus_git_dir_path(&self.git_dir))
            .context("Failed to create the directory to house the outlining tree")?;

        // Add the worktree
        {
            let (mut cmd, scmd) = git_helper::git_command(self.app.clone())?;
            let cmd = cmd
                .current_dir(&self.path)
                .arg("worktree")
                .arg("add")
                .arg("--no-checkout")
                .arg(&path)
                .arg("HEAD");
            scmd.ensure_success_or_log(cmd, SandboxCommandOutput::Stderr)
                .context("git worktree add failed")?;
        }

        let working_tree =
            WorkingTree::new(git2::Repository::open(self.outlining_tree_git_dir())?)?;
        let outlining_tree = OutliningTreeOutliner::new(Arc::new(working_tree));
        let commit_id = self.get_head_commit()?.id();
        outlining_tree.apply_configured_outlining_patterns(commit_id, self.app.clone())?;
        Ok(())
    }

    pub fn repair_outlining_tree(&self) -> Result<()> {
        let path = Self::outlining_tree_path(&self.git_dir);
        // repair the worktree
        let (mut cmd, scmd) = git_helper::git_command(self.app.clone())?;
        let cmd = cmd
            .current_dir(&self.path)
            .arg("worktree")
            .arg("repair")
            .arg(&path);
        scmd.ensure_success_or_log(cmd, SandboxCommandOutput::Stderr)
            .context("git worktree repair failed")?;

        Ok(())
    }

    fn outlining_tree_git_dir(&self) -> PathBuf {
        self.git_dir.join("worktrees").join(OUTLINING_TREE_NAME)
    }

    pub fn create_working_tree(&self) -> Result<()> {
        // Apply the top-level patterns
        let working_tree = WorkingTree::from_git_dir(&self.git_dir)?;
        working_tree
            .apply_working_tree_patterns(self.app.clone())
            .context("Failed to apply top-level patterns")?;
        Ok(())
    }

    /// Get a reference to the repo's outlining tree.
    pub fn outliner(&self) -> Option<Arc<dyn Outliner>> {
        self.outliner.clone()
    }

    pub fn dense_outlining_tree(&self) -> Result<OutliningTreeOutliner> {
        todo!("impl")
    }

    /// Get a reference to the working tree
    pub fn working_tree(&self) -> Result<Arc<WorkingTree>> {
        Ok(self
            .working_tree
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Repository has no working tree"))
            .context("A working tree is required")?
            .clone())
    }

    /// Get a reference to the repo's git dir.
    pub fn git_dir(&self) -> &PathBuf {
        &self.git_dir
    }

    /// Returns $GIT_DIR/focus
    pub fn git_focus_dir(&self) -> PathBuf {
        self.git_dir().join("focus")
    }

    /// Returns $GIT_DIR/focus/project-cache
    pub fn project_cache_dir(&self) -> PathBuf {
        self.git_focus_dir().join("project-cache")
    }

    /// Write git config to support gitstats instrumentation.
    /// This sets `focus.version` and `twitter.statsenabled`
    pub fn write_git_config_to_support_instrumentation(&self) -> Result<()> {
        if cfg!(feature = "twttr") {
            const VERSION_CONFIG_KEY: &str = "focus.version";
            const GITSTATS_CONFIG_KEY: &str = "twitter.statsenabled";
            self.repo
                .config()?
                .set_str(VERSION_CONFIG_KEY, env!("CARGO_PKG_VERSION"))?;

            self.repo.config()?.set_bool(GITSTATS_CONFIG_KEY, true)?;
        }
        Ok(())
    }

    pub fn selection_manager(&self) -> Result<SelectionManager> {
        SelectionManager::from_repo(self)
    }

    // We expose the computed selection here for use in benchmarks since `SelectionManager` exposes types not visible outside the crate.
    pub fn computed_selection(&self) -> Result<Selection> {
        self.selection_manager()?.computed_selection()
    }

    pub fn get_prefetch_head_commit(
        &self,
        remote_name: &str,
        branch_name: &str,
    ) -> Result<Option<git2::Commit>> {
        let ref_name = format!("refs/prefetch/remotes/{}/{}", remote_name, branch_name);
        match self.repo.find_reference(&ref_name) {
            Ok(prefetch_head_reference) => Ok(Some(
                prefetch_head_reference
                    .peel_to_commit()
                    .context("Resolving commit")?,
            )),
            Err(e) => {
                warn!(?ref_name, ?e, "Could not find prefetch head commit",);
                Ok(None)
            }
        }
    }

    pub fn get_head_commit(&self) -> Result<git2::Commit> {
        let head_reference = self.repo.head().context("resolving HEAD reference")?;
        let head_commit = head_reference
            .peel_to_commit()
            .context("resolving HEAD commit")?;
        Ok(head_commit)
    }

    /// Read whether preemptive sync is enabled from Git config.
    pub fn get_preemptive_sync_enabled(&self) -> Result<bool> {
        let snapshot = self
            .underlying()
            .config()
            .context("Reading config")?
            .snapshot()
            .context("Snapshotting config")?;

        Ok(snapshot
            .get_bool(PREEMPTIVE_SYNC_ENABLED_CONFIG_KEY)
            .unwrap_or(false))
    }

    /// Set whether preemptive sync is enabled in the Git config.
    pub fn set_preemptive_sync_enabled(&self, enabled: bool) -> Result<()> {
        let working_tree = self.working_tree()?;

        git_helper::write_config(
            working_tree.work_dir(),
            PREEMPTIVE_SYNC_ENABLED_CONFIG_KEY,
            enabled.to_string().as_str(),
            self.app.clone(),
        )
        .context("Writing preemptive sync enabled key")?;

        Ok(())
    }

    /// Read the configured preemptive sync idle threshold duration. This indicates how long the computer must be inactive to allow for preemptive sync to run.
    pub fn get_preemptive_sync_idle_threshold(&self) -> Result<Duration> {
        let snapshot = self
            .underlying()
            .config()
            .context("Reading config")?
            .snapshot()
            .context("Snapshotting config")?;

        let mut threshold = snapshot
            .get_i32(PREEMPTIVE_SYNC_USER_IDLE_MILLIS_THRESHOLD_CONFIG_KEY)
            .unwrap_or(PREEMPTIVE_SYNC_USER_IDLE_MILLIS_THRESHOLD_DEFAULT);
        if threshold < 1 {
            warn!(
                "Configuration value of '{}' must be positive; using default ({})",
                PREEMPTIVE_SYNC_USER_IDLE_MILLIS_THRESHOLD_CONFIG_KEY,
                PREEMPTIVE_SYNC_USER_IDLE_MILLIS_THRESHOLD_DEFAULT
            );
            threshold = PREEMPTIVE_SYNC_USER_IDLE_MILLIS_THRESHOLD_DEFAULT;
        }
        Ok(Duration::from_millis(threshold as u64))
    }

    /// Write the configured preemptive sync idle threshold duration.
    pub fn set_preemptive_sync_idle_threshold(&self, duration: Duration) -> Result<()> {
        let working_tree = self.working_tree()?;

        git_helper::write_config(
            working_tree.work_dir(),
            PREEMPTIVE_SYNC_USER_IDLE_MILLIS_THRESHOLD_CONFIG_KEY,
            &duration.as_millis().to_string(),
            self.app.clone(),
        )
        .context("Writing preemptive sync enabled key")?;

        Ok(())
    }

    pub fn primary_branch_name(&self) -> Result<String> {
        let repo = self.underlying();
        if repo.find_reference("refs/heads/master").is_ok() {
            Ok(String::from("master"))
        } else if repo.find_reference("refs/heads/main").is_ok() {
            Ok(String::from("main"))
        } else {
            bail!("Could not determine primary branch name");
        }
    }

    pub fn get_project_cache_remote_endpoint(&self) -> Result<Option<Url>> {
        let config_snapshot = self.repo.config()?.snapshot()?;
        match config_snapshot.get_str(PROJECT_CACHE_ENDPOINT_CONFIG_KEY) {
            Ok(endpoint) => Url::parse(endpoint)
                .with_context(|| {
                    format!(
                        "Failed parsing {} '{}'",
                        PROJECT_CACHE_ENDPOINT_CONFIG_KEY, endpoint
                    )
                })
                .map(Some),
            Err(_) => Ok(None),
        }
    }

    pub fn get_project_cache_include_header_file(&self) -> Result<Option<String>> {
        let config_snapshot = self.repo.config()?.snapshot()?;
        Ok(config_snapshot
            .get_str(PROJECT_CACHE_INCLUDE_HEADERS_FILE_CONFIG_KEY)
            .ok()
            .map(|s| s.to_owned()))
    }

    pub fn set_project_cache_include_header_file(&self, value: &str) -> Result<()> {
        let working_tree = self.working_tree()?;

        git_helper::write_config(
            working_tree.work_dir(),
            PROJECT_CACHE_INCLUDE_HEADERS_FILE_CONFIG_KEY,
            value,
            self.app.clone(),
        )
        .with_context(|| {
            format!(
                "Writing key '{}'",
                PROJECT_CACHE_INCLUDE_HEADERS_FILE_CONFIG_KEY
            )
        })?;

        Ok(())
    }

    pub fn get_bazel_oneshot_resolution(&self) -> Result<bool> {
        let mut config_snapshot = self.repo.config()?.snapshot()?;
        config_snapshot.get_bool_with_default(BAZEL_ONE_SHOT_RESOLUTION_CONFIG_KEY, false)
    }

    pub fn set_bazel_oneshot_resolution(&self, value: bool) -> Result<()> {
        git_helper::write_config(
            &self.path,
            BAZEL_ONE_SHOT_RESOLUTION_CONFIG_KEY,
            value.to_string().as_str(),
            self.app.clone(),
        )
    }
}
