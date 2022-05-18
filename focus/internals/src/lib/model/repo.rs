use focus_util::{
    app::App,
    git_helper::{self, get_head_commit},
    paths,
    sandbox_command::SandboxCommandOutput,
};
use std::{
    collections::HashSet,
    fs::{self},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use crate::{
    hashing,
    index::{
        get_files_to_materialize, update_object_database_from_resolution, DependencyKey,
        HashContext, ObjectDatabase, PathsToMaterializeResult,
    },
    model::outlining::{LeadingPatternInserter, Pattern},
    target::TargetSet,
    target_resolver::{
        CacheOptions, ResolutionRequest, ResolutionResult, Resolver, RoutingResolver,
    },
};

use super::{
    outlining::{PatternContainer, PatternSet, PatternSetWriter, BASELINE_PATTERNS},
    selection::{Selection, SelectionManager},
};

use anyhow::{bail, Context, Result};
use git2::{Oid, Repository, TreeWalkMode, TreeWalkResult};
use tracing::{debug, info, info_span, trace};
use uuid::Uuid;

const SYNC_REF_NAME: &str = "refs/focus/sync";
const UUID_CONFIG_KEY: &str = "focus.uuid";
const VERSION_CONFIG_KEY: &str = "focus.version";
const GITSTATS_CONFIG_KEY: &str = "twitter.statsenabled";
const OUTLINING_PATTERN_FILE_NAME: &str = "focus/outlining.patterns.json";
const LAST: usize = usize::MAX;

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

    fn info_dir(&self) -> PathBuf {
        self.repo.path().join("info")
    }

    /// The location of the current sparse checkout file within the working tree.
    fn sparse_checkout_path(&self) -> PathBuf {
        self.info_dir().join("sparse-checkout")
    }

    /// Writes the given `patterns` to the working tree.
    pub fn apply_sparse_patterns(
        &self,
        patterns: &PatternSet,
        cone: bool,
        app: Arc<App>,
    ) -> Result<bool> {
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
            let existing_content_hash = hashing::hash_file_lines(&sparse_profile_path)
                .context("Hashing contents of existing sparse profile failed")?;
            if !existing_content_hash.is_empty() && existing_content_hash == new_content_hash {
                // We wrote the exact same thing. Skip everything.
                info!(profile = ?sparse_profile_path, "Skipping application of the sparse profile because it has not changed");
                std::fs::remove_file(&candidate_sparse_profile_path)
                    .context("Removing candidate sparse profile")?;
                return Ok(false);
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
            let description = format!("Running git {:?} in {}", args, self.work_dir().display());
            let (mut cmd, scmd) = git_helper::git_command(description.clone(), app.clone())?;
            scmd.ensure_success_or_log(
                cmd.current_dir(self.work_dir()).args(args),
                SandboxCommandOutput::Stderr,
                &description,
            )
            .with_context(|| format!("In working tree {}", self.work_dir().display()))
            .context("git sparse-checkout init failed")?;
        }

        // Newer versions of Git don't actually check out files when `sparse-checkout init` runs, so run `git checkout`. It might be worth making this behavior version-dependent.
        info!("Checking out");
        {
            let args = vec!["checkout"];
            let description = format!("Running git {:?} in {}", args, self.work_dir().display());
            let (mut cmd, scmd) = git_helper::git_command(description.clone(), app)?;
            scmd.ensure_success_or_log(
                cmd.current_dir(self.work_dir()).args(args),
                SandboxCommandOutput::Stderr,
                &description,
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

        let description = format!("Running git {:?} in {}", args, self.work_dir().display());
        let (mut cmd, scmd) = git_helper::git_command(description.clone(), app)?;
        scmd.ensure_success_or_log(
            cmd.current_dir(self.work_dir()).args(args),
            SandboxCommandOutput::Stderr,
            &description,
        )
        .context("git switch failed")?;

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
        Ok(BASELINE_PATTERNS.clone())
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
        self.apply_sparse_patterns(&patterns, true, app)
            .context("Failed to apply root-only patterns")
    }

    /// Reads the commit ID of the sparse sync ref (named SYNC_REF_NAME)
    pub fn read_sync_point_ref(&self) -> Result<Option<Oid>> {
        let description = format!(
            "Recording sparse sync point in {}",
            self.work_dir().display()
        );
        let reference = self
            .repo
            .find_reference(SYNC_REF_NAME)
            .context(description.clone())
            .context("Finding sync reference");
        match reference {
            Ok(reference) => {
                let commit = reference
                    .peel_to_commit()
                    .context(description)
                    .context("Finding commit associated with reference")?;
                Ok(Some(commit.id()))
            }
            _ => Ok(None),
        }
    }

    /// Updates the sparse sync ref to the value of the HEAD ref (named SYNC_REF_NAME)
    pub fn write_sync_point_ref(&self) -> Result<()> {
        let description = format!(
            "Recording sparse sync point in {}",
            self.work_dir().display()
        );
        let head_commit = self.get_head_commit().context(description)?;
        self.repo
            .reference(SYNC_REF_NAME, head_commit.id(), true, "focus sync")?;

        Ok(())
    }

    pub fn git_repo(&self) -> &Repository {
        &self.repo
    }

    /// Determine if the working tree is clean
    pub fn is_clean(&self, app: Arc<App>) -> Result<bool> {
        Ok(git_helper::run_consuming_stdout(
            "git status",
            self.work_dir(),
            vec!["status", "--porcelain"],
            app,
        )?
        .trim()
        .is_empty())
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
}

/// A specialization of a WorkingTree used for outlining tasks, containing only files related to, and necessary for querying, the build graph.
#[derive(Debug, PartialEq, Eq)]
pub struct OutliningTree {
    underlying: Arc<WorkingTree>,
}

impl OutliningTree {
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
            .apply_sparse_patterns(&patterns, false, app)
            .context("Failed to apply build file patterns")
    }

    /// Read configured outlining patterns from the repository at the given commit.
    fn configured_outlining_patterns(&self, commit_id: git2::Oid) -> Result<PatternSet> {
        let repo = self.underlying();
        let repo = repo.git_repo();
        let commit = repo.find_commit(commit_id).context("Resolving commit")?;
        let tree = commit.tree().context("Resolving tree")?;
        let pattern_file = tree
            .get_path(Path::new(OUTLINING_PATTERN_FILE_NAME)).with_context(|| format!(
                "No outlining pattern file (named '{}') was found in the repository at this commit ({})",
                OUTLINING_PATTERN_FILE_NAME,
                &commit.id().to_string(),
            ))?;
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

    fn resolver(&self) -> Result<RoutingResolver> {
        let cache_dir = dirs::cache_dir()
            .context("failed to determine cache dir")?
            .join("focus")
            .join("cache");
        Ok(RoutingResolver::new(cache_dir.as_path()))
    }

    pub fn outline(
        &self,
        commit_id: git2::Oid,
        target_set: &TargetSet,
        app: Arc<App>,
    ) -> Result<(PatternSet, ResolutionResult)> {
        self.apply_configured_outlining_patterns(commit_id, app.clone())
            .context("Applying configured outlining patterns failed")?;
        self.underlying()
            .switch_to_commit(commit_id, true, true, app.clone())
            .context("Failed to switch to commit")?;

        let mut patterns = PatternSet::new();

        let repo_path = self.underlying().work_dir().to_owned();
        let cache_options = CacheOptions::default();
        let request = ResolutionRequest {
            repo: repo_path.clone(),
            targets: target_set.clone(),
        };
        let resolver = self.resolver().context("Failed to create resolver")?;
        let result = resolver.resolve(&request, &cache_options, app)?;

        let treat_path = |p: &Path| -> Result<Option<PathBuf>> {
            let p = p
                .strip_prefix(&repo_path)
                .context("Failed to strip repo path prefix")?;
            if p == paths::MAIN_SEPARATOR_PATH.as_path() {
                Ok(None)
            } else {
                Ok(Some(p.to_owned()))
            }
        };

        for path in result.paths.iter() {
            let qualified_path = repo_path.join(path);
            match paths::find_closest_directory_with_build_file(&qualified_path, &repo_path)
                .context("locating closest build file")?
            {
                Some(path_to_closest_build_file) => {
                    debug!(
                        "Adding directory with closest build definition: {}",
                        path_to_closest_build_file.display()
                    );
                    if let Some(path) = treat_path(&path_to_closest_build_file)? {
                        patterns.insert_leading(Pattern::Directory {
                            precedence: LAST,
                            path,
                            recursive: true,
                        });
                    }
                }
                None => {
                    debug!("Adding directory verbatim: {}", qualified_path.display());
                    if let Some(path) = treat_path(&qualified_path)? {
                        patterns.insert(Pattern::Directory {
                            precedence: LAST,
                            path,
                            recursive: true,
                        });
                    }
                }
            }
        }

        Ok((patterns, result))
    }
}

const OUTLINING_TREE_NAME: &str = "outlining-tree";

pub struct Repo {
    path: PathBuf,
    git_dir: PathBuf,
    working_tree: Option<WorkingTree>,
    outlining_tree: Option<OutliningTree>,
    repo: git2::Repository,
    app: Arc<App>,
}

impl Repo {
    pub fn open(path: &Path, app: Arc<App>) -> Result<Self> {
        let repo = git2::Repository::open(&path)
            .with_context(|| format!("Opening repo {}", path.display()))
            .context("Failed to open repo")?;
        if repo.is_bare() {
            bail!("Bare repos are not supported");
        }
        let git_dir = repo.path().to_owned();
        let working_tree = match repo.workdir() {
            Some(work_dir) => {
                let repo = git2::Repository::open(work_dir)?;
                Some(WorkingTree::new(repo)?)
            }
            None => None,
        };

        let outlining_tree_path = Self::outlining_tree_path(&git_dir);
        let outlining_tree_git_dir = git_dir.join("worktrees").join(OUTLINING_TREE_NAME);
        let outlining_tree = if outlining_tree_path.is_dir() {
            Some(OutliningTree::new(Arc::new(WorkingTree::from_git_dir(
                &outlining_tree_git_dir,
            )?)))
        } else {
            None
        };
        let path = path.to_owned();

        Ok(Self {
            path,
            git_dir,
            working_tree,
            outlining_tree,
            repo,
            app,
        })
    }

    pub fn underlying(&self) -> &git2::Repository {
        &self.repo
    }

    pub fn focus_git_dir_path(git_dir: &Path) -> PathBuf {
        git_dir.join("focus")
    }

    pub fn outlining_tree_path(git_dir: &Path) -> PathBuf {
        Self::focus_git_dir_path(git_dir).join(OUTLINING_TREE_NAME)
    }

    /// Run a sync, returning the number of patterns that were applied and whether a checkout occured as a result of the profile changing.
    pub fn sync(
        &self,
        targets: &TargetSet,
        skip_pattern_application: bool,
        app: Arc<App>,
        odb: &dyn ObjectDatabase,
    ) -> Result<(usize, bool)> {
        let head_commit = self.get_head_commit()?;
        let head_tree = head_commit.tree().context("Failed to resolve head tree")?;
        let hash_context = HashContext {
            repo: &self.repo,
            head_tree: &head_tree,
            caches: Default::default(),
        };

        let (working_tree, outlining_tree) = match (&self.working_tree, &self.outlining_tree) {
            (Some(working_tree), Some(outlining_tree)) => (working_tree, outlining_tree),
            _ => {
                // TODO: we might succeed in synchronization without an outlining tree.
                bail!("Synchronization is only possible in a repo with both working and outlining trees");
            }
        };

        let mut outline_patterns = {
            info!("Checking cache for sparse checkout patterns");
            let paths_to_materialize = get_files_to_materialize(
                &hash_context,
                odb,
                targets.iter().cloned().map(DependencyKey::from).collect(),
            )?;

            match paths_to_materialize {
                PathsToMaterializeResult::Ok { seen_keys, paths } => {
                    info!(
                        num_seen_keys = seen_keys.len(),
                        "Cache hit for sparse checkout patterns"
                    );
                    paths
                        .into_iter()
                        .map(|path| Pattern::Directory {
                            precedence: LAST,
                            path,
                            recursive: true,
                        })
                        .collect()
                }

                PathsToMaterializeResult::MissingKeys { keys: missing_keys } => {
                    info!(
                        num_missing_keys = ?missing_keys.len(),
                        "Cache miss for sparse checkout patterns; querying Bazel"
                    );
                    debug!(?missing_keys, "These are the missing keys");
                    let (outline_patterns, resolution_result) = outlining_tree
                        .outline(head_commit.id(), targets, app.clone())
                        .context("Failed to outline")?;

                    debug!(?resolution_result, ?outline_patterns, "Resolved patterns");
                    update_object_database_from_resolution(&hash_context, odb, &resolution_result)?;
                    outline_patterns
                }
            }
        };

        trace!(?outline_patterns);
        outline_patterns.extend(working_tree.default_working_tree_patterns()?);
        let checked_out = if skip_pattern_application {
            false
        } else {
            working_tree
                .apply_sparse_patterns(&outline_patterns, true, app)
                .context("Failed to apply outlined patterns to working tree")?
        };
        Ok((outline_patterns.len(), checked_out))
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
            let description = format!("Creating outlining tree worktree in {}", path.display());
            let (mut cmd, scmd) = git_helper::git_command(description.clone(), self.app.clone())?;
            let cmd = cmd
                .current_dir(&self.path)
                .arg("worktree")
                .arg("add")
                .arg("--no-checkout")
                .arg(&path)
                .arg("HEAD");
            scmd.ensure_success_or_log(cmd, SandboxCommandOutput::Stderr, &description)
                .context("git worktree add failed")
                .map(|_| ())?;
        }

        let working_tree = WorkingTree::new(git2::Repository::open(self.working_tree_git_dir())?)?;
        let outlining_tree = OutliningTree::new(Arc::new(working_tree));
        let commit_id = self.get_head_commit()?.id();
        outlining_tree.apply_configured_outlining_patterns(commit_id, self.app.clone())?;
        Ok(())
    }

    fn working_tree_git_dir(&self) -> PathBuf {
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
    pub fn outlining_tree(&self) -> Option<&OutliningTree> {
        self.outlining_tree.as_ref()
    }

    /// Get a reference to the repo's working tree.
    pub fn working_tree(&self) -> Option<&WorkingTree> {
        self.working_tree.as_ref()
    }

    /// Get a reference to the repo's git dir.
    pub fn git_dir(&self) -> &PathBuf {
        &self.git_dir
    }

    /// Write git config to support gitstats instrumentation.
    /// This sets `focus.version` and `twitter.statsenabled`
    pub fn write_git_config_to_support_instrumentation(&self) -> Result<()> {
        self.repo
            .config()?
            .set_str(VERSION_CONFIG_KEY, env!("CARGO_PKG_VERSION"))?;

        self.repo.config()?.set_bool(GITSTATS_CONFIG_KEY, true)?;
        Ok(())
    }

    pub(crate) fn selection_manager(&self) -> Result<SelectionManager> {
        SelectionManager::from_repo(self)
    }

    // We expose the computed selection here for use in benchmarks since `SelectionManager` exposes types not visible outside the crate.
    pub fn computed_selection(&self) -> Result<Selection> {
        self.selection_manager()?.computed_selection()
    }

    pub fn get_head_commit(&self) -> Result<git2::Commit> {
        let head_reference = self.repo.head().context("resolving HEAD reference")?;
        let head_commit = head_reference
            .peel_to_commit()
            .context("resolving HEAD commit")?;
        Ok(head_commit)
    }
}
