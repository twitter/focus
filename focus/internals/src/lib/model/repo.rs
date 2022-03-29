use focus_util::{app::App, git_helper, paths, sandbox_command::SandboxCommandOutput};
use std::{
    collections::HashSet,
    fs::{self},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use crate::{
    coordinate::CoordinateSet,
    coordinate_resolver::{CacheOptions, ResolutionRequest, Resolver, RoutingResolver},
    model::outlining::{LeadingPatternInserter, Pattern},
};

use super::{
    layering::LayerSets,
    outlining::{PatternSet, PatternSetWriter, BUILD_FILE_PATTERNS, SOURCE_BASELINE_PATTERNS},
};

use anyhow::{bail, Context, Result};
use git2::{Oid, Repository, TreeWalkMode, TreeWalkResult};
use tracing::{debug, info, info_span};
use uuid::Uuid;

const SYNC_REF_NAME: &str = "refs/focus/sync";
const UUID_CONFIG_KEY: &str = "focus.uuid";
const VERSION_CONFIG_KEY: &str = "focus.version";
const GITSTATS_CONFIG_KEY: &str = "twitter.statsenabled";

/// Models a Git working tree.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct WorkingTree {
    path: PathBuf,
    git_dir: PathBuf,
}

impl WorkingTree {
    /// Creates an instance.
    pub fn new(path: PathBuf, git_dir: PathBuf) -> Self {
        Self { path, git_dir }
    }

    fn info_dir(&self) -> PathBuf {
        self.git_dir.join("info")
    }

    /// The location of the current sparse checkout file within the working tree.
    fn sparse_checkout_path(&self) -> PathBuf {
        self.info_dir().join("sparse-checkout")
    }

    /// Writes the given `patterns` to the working tree. If `cone` is set, the
    pub fn apply_sparse_patterns(
        &self,
        patterns: &PatternSet,
        _cone: bool,
        app: Arc<App>,
    ) -> Result<()> {
        // Write the patterns
        let info_dir = self.info_dir();
        std::fs::create_dir_all(&info_dir)
            .with_context(|| format!("In working tree {}", self.path.display()))
            .context("Failed to create leading directories for sparse profile")?;

        // Make sure Git doesn't try to do something cute with the sparse profile like merge existing contents.
        let sparse_profile_path = self.sparse_checkout_path();
        if sparse_profile_path.is_file() {
            std::fs::remove_file(&sparse_profile_path)
                .context("Could not remove existing sparse profile")?;
        }
        patterns.write_to_file(&sparse_profile_path)?;

        // Update the working tree to match
        info!(count = %patterns.len(), "Applying patterns");
        {
            let cone = false; // Cone patterns are disabled.
            let args = vec![
                "sparse-checkout",
                "init",
                if cone { "--cone" } else { "--no-cone" },
            ];
            let description = format!("Running git {:?} in {}", args, self.path.display());
            let (mut cmd, scmd) = git_helper::git_command(description.clone(), app)?;
            scmd.ensure_success_or_log(
                cmd.current_dir(&self.path).args(args),
                SandboxCommandOutput::Stderr,
                &description,
            )
            .with_context(|| format!("In working tree {}", self.path.display()))
            .context("git sparse-checkout set failed")?;
        }

        Ok(())
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
        let span = info_span!("Switching to detached commit", commit_id = %commit_id_str, path = ?self.path);
        let _guard = span.enter();

        let mut args = vec!["switch", "--ignore-other-worktrees"];
        if detach {
            args.push("--detach");
        }
        if discard_changes {
            args.push("--discard-changes");
        }
        args.push(&commit_id_str);

        let description = format!("Running git {:?} in {}", args, self.path.display());
        let (mut cmd, scmd) = git_helper::git_command(description.clone(), app)?;
        scmd.ensure_success_or_log(
            cmd.current_dir(&self.path).args(args),
            SandboxCommandOutput::Stderr,
            &description,
        )
        .context("git switch failed")?;

        Ok(())
    }

    /// Get a reference to the working tree's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get a reference to the working tree's git dir.
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    pub fn default_working_tree_patterns(&self) -> Result<PatternSet> {
        Ok(SOURCE_BASELINE_PATTERNS.clone())
    }

    #[allow(dead_code)]
    fn locate_paths_in_tree(&self, prefixes: &HashSet<PathBuf>) -> Result<PatternSet> {
        let mut results = PatternSet::new();
        let repo = self
            .git_repo()
            .with_context(|| (format!("Opening repo in {}", self.path.display())))?;
        let head_tree = repo
            .head()
            .context("Failed to resolve the HEAD reference")?
            .peel_to_tree()
            .context("Failed to locate the tree associated with the HEAD reference")?;
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
                    });
                    TreeWalkResult::Ok
                } else {
                    TreeWalkResult::Skip
                }
            })
            .context("Failed to walk tree")?;

        Ok(results)
    }

    fn apply_working_tree_patterns(&self, app: Arc<App>) -> Result<()> {
        let patterns = self.default_working_tree_patterns()?;
        self.apply_sparse_patterns(&patterns, true, app)
            .context("Failed to apply root-only patterns")
    }

    /// Reads the commit ID of the sparse sync ref (named SYNC_REF_NAME)
    pub fn read_sync_point_ref(&self) -> Result<Option<Oid>> {
        let description = format!("Recording sparse sync point in {}", self.path.display());
        let repo = self.git_repo()?;
        let reference = repo
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
        let description = format!("Recording sparse sync point in {}", self.path.display());
        let git_repo = self.git_repo()?;
        let head_commit = git_repo
            .head()
            .context(description.clone())
            .context("Reading HEAD reference")?
            .peel_to_commit()
            .context(description)
            .context("Finding commit associated with HEAD reference")?;
        git_repo.reference(SYNC_REF_NAME, head_commit.id(), true, "focus sync")?;

        Ok(())
    }

    // TODO: Try to buffer instantiation of these.
    pub fn git_repo(&self) -> Result<Repository> {
        Ok(Repository::open(self.path())?)
    }

    /// Determine if the working tree is clean
    pub fn is_clean(&self, app: Arc<App>) -> Result<bool> {
        Ok(git_helper::run_consuming_stdout(
            "git status",
            &self.path,
            vec!["status", "--porcelain"],
            app,
        )?
        .trim()
        .is_empty())
    }

    /// Retrieve the LayerSets model
    pub fn layer_sets(&self) -> Result<LayerSets> {
        Ok(LayerSets::new(&self.path))
    }

    pub fn read_uuid(&self) -> Result<Option<Uuid>> {
        let repo = self.git_repo()?;
        let config_snapshot = repo.config()?.snapshot()?;
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
        let repo = self.git_repo()?;
        repo.config()?
            .set_str(UUID_CONFIG_KEY, uuid.to_string().as_str())?;
        Ok(uuid)
    }
}

/// A specialization of a WorkingTree used for outlining tasks, containing only files related to, and necessary for querying, the build graph.
#[derive(Debug, Hash, PartialEq, Eq)]
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

    pub fn build_file_patterns(&self) -> Result<PatternSet> {
        let mut pattern_set = self.underlying.default_working_tree_patterns()?;
        pattern_set.extend(BUILD_FILE_PATTERNS.clone());
        Ok(pattern_set)
    }

    pub fn apply_build_file_patterns(&self, app: Arc<App>) -> Result<()> {
        let patterns = self.build_file_patterns()?;
        self.underlying
            .apply_sparse_patterns(&patterns, false, app)
            .context("Failed to apply build file patterns")
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
        coordinate_set: &CoordinateSet,
        app: Arc<App>,
    ) -> Result<PatternSet> {
        // Switch to the commit
        self.underlying()
            .switch_to_commit(commit_id, true, true, app.clone())
            .context("Failed to switch to commit")?;

        let mut patterns = PatternSet::new();

        let repo_path = self.underlying().path().to_owned();
        let cache_options = CacheOptions::default();
        let request = ResolutionRequest {
            repo: repo_path.clone(),
            coordinate_set: coordinate_set.clone(),
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

        const LAST: usize = usize::MAX;
        for path in result.paths {
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
                        });
                    }
                }
                None => {
                    debug!("Adding directory verbatim: {}", qualified_path.display());
                    if let Some(path) = treat_path(&qualified_path)? {
                        if let Some(fragment) = path.to_str() {
                            patterns.insert(Pattern::Verbatim {
                                precedence: LAST,
                                fragment: fragment.to_owned(),
                            });
                        } else {
                            bail!("Path {} not representable as UTF-8", path.display());
                        }
                    }
                }
            }
        }

        Ok(patterns)
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
        let working_tree = repo
            .workdir()
            .map(|work_tree_path| WorkingTree::new(work_tree_path.to_owned(), git_dir.clone()));

        let outlining_tree_path = Self::outlining_tree_path(&git_dir);
        let outlining_tree_git_dir = git_dir.join("worktrees").join(OUTLINING_TREE_NAME);
        let outlining_tree = if outlining_tree_path.is_dir() {
            Some(OutliningTree::new(Arc::new(WorkingTree::new(
                outlining_tree_path,
                outlining_tree_git_dir,
            ))))
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

    pub fn focus_git_dir_path(git_dir: &Path) -> PathBuf {
        git_dir.join("focus")
    }

    pub fn outlining_tree_path(git_dir: &Path) -> PathBuf {
        Self::focus_git_dir_path(git_dir).join(OUTLINING_TREE_NAME)
    }

    /// Run a sync, returning the number of patterns that were applied.
    pub fn sync(&self, coordinates: &CoordinateSet, app: Arc<App>) -> Result<usize> {
        match (&self.working_tree, &self.outlining_tree) {
            (Some(working_tree), Some(outlining_tree)) => {
                // Get the HEAD commit ID for the repo so that we can outline using the same commit.
                let head_commit = self
                    .repo
                    .head()
                    .context("Failed to resolve HEAD reference")?
                    .peel_to_commit()
                    .context("Failed to peel to commit")?;

                let mut outline_patterns: PatternSet = outlining_tree
                    .outline(head_commit.id(), coordinates, app.clone())
                    .context("Failed to outline")?;
                debug!(?outline_patterns);
                outline_patterns.extend(working_tree.default_working_tree_patterns()?);
                working_tree
                    .apply_sparse_patterns(&outline_patterns, true, app)
                    .context("Failed to apply outlined patterns to working tree")?;
                Ok(outline_patterns.len())
            }
            _ => {
                bail!("Synchronization is only possible in a repo with both working and outlining trees")
            }
        }
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

        let work_tree_gitdir = self.working_tree_git_dir();
        // Apply the correct sparse patterns
        let outlining_tree = OutliningTree::new(Arc::new(WorkingTree::new(path, work_tree_gitdir)));
        outlining_tree.apply_build_file_patterns(self.app.clone())
    }

    fn working_tree_git_dir(&self) -> PathBuf {
        self.git_dir.join("worktrees").join(OUTLINING_TREE_NAME)
    }

    pub fn create_working_tree(&self) -> Result<()> {
        let path = self
            .git_dir
            .parent()
            .with_context(|| {
                format!(
                    "Failed determining the parent directory of git_dir ({})",
                    self.git_dir.display(),
                )
            })?
            .to_owned();

        // Apply the top-level patterns
        let working_tree = WorkingTree::new(path, self.git_dir.to_owned());
        working_tree
            .apply_working_tree_patterns(self.app.clone())
            .context("Failed to apply top-level patterns")
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
}
