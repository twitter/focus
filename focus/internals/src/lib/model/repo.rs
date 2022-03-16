use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::Stdio,
    str::FromStr,
    sync::Arc,
};

use crate::{
    app::App,
    coordinate::CoordinateSet,
    coordinate_resolver::{CacheOptions, ResolutionRequest, Resolver, RoutingResolver},
    model::outlining::{Pattern, PatternSetFilter},
    util::{git_helper, paths, sandbox_command::SandboxCommandOutput},
};

use super::{
    layering::LayerSets,
    outlining::{PatternSet, PatternSetWriter, BUILD_FILE_PATTERNS, SOURCE_BASELINE_PATTERNS},
};

use anyhow::{bail, Context, Result};
use git2::{Oid, Repository};
use tracing::{debug, info_span};
use uuid::Uuid;

const SYNC_REF_NAME: &str = "refs/focus/sync";
const UUID_CONFIG_KEY: &str = "focus.uuid";

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

    /// The location of the candidate sparse checkout file within the working tree.
    fn sparse_checkout_file_path(&self) -> PathBuf {
        self.git_dir.join("info").join("sparse-checkout.focus")
    }

    /// Writes the given `patterns` to the working tree. If `cone` is set, the
    pub fn apply_sparse_patterns(
        &self,
        patterns: &PatternSet,
        cone: bool,
        app: Arc<App>,
    ) -> Result<()> {
        // Write the patterns

        let sparse_profile_path = self.sparse_checkout_file_path();
        let parent_path = sparse_profile_path
            .parent()
            .with_context(|| format!("In working tree {}", self.path.display()))
            .context("Could not determine parent of sparse checkout file")?;
        std::fs::create_dir_all(parent_path)
            .with_context(|| format!("In working tree {}", self.path.display()))
            .context("Failed to create leading directories for sparse profile")?;

        let first_run = !sparse_profile_path.exists();

        if !first_run {
            let mut backup_path = sparse_profile_path.clone();
            backup_path.set_extension("focus.previous");
            std::fs::rename(&sparse_profile_path, &backup_path)
                .with_context(|| format!("In working tree {}", self.path.display()))
                .context("Failed to delete existing sparse profile")?;
        }
        patterns.write_to_file(&sparse_profile_path)?;

        // Update the working tree to match
        let span = info_span!("Applying the sparse patterns", path = ?self.path);
        let _guard = span.enter();

        if first_run {
            let mut args = vec!["sparse-checkout", "init"];
            if cone {
                // TODO: Reconsider cone patterns
                args.push("--cone");
            }

            let description = format!("Running git {:?} in {}", args, self.path.display());
            let (mut cmd, scmd) = git_helper::git_command(description.clone(), app.clone())?;
            scmd.ensure_success_or_log(
                cmd.current_dir(&self.path).args(args),
                SandboxCommandOutput::Stderr,
                &description,
            )
            .with_context(|| format!("In working tree {}", self.path.display()))
            .context("git sparse-checkout init failed")?;
        } else {
            let repo = self
                .git_repo()
                .context("Opening the repo in the working tree")
                .with_context(|| format!("In working tree {}", self.path.display()))?;
            let mut config = repo
                .config()
                .context("Loading config")
                .with_context(|| format!("In working tree {}", self.path.display()))?;
            config
                .set_bool("core.spareCheckoutCone", cone)
                .context("Setting config")
                .with_context(|| format!("In working tree {}", self.path.display()))?;
        }

        {
            let file = File::open(&sparse_profile_path)
                .context("Failed to open the sparse pattern file")?;
            let span = info_span!("Setting sparse patterns", path = ?sparse_profile_path);
            let _guard = span.enter();
            let args = vec!["sparse-checkout", "set", "--stdin"];
            let description = format!("Running git {:?} in {}", args, self.path.display());
            let (mut cmd, scmd) = git_helper::git_command(description.clone(), app.clone())?;
            scmd.ensure_success_or_log(
                cmd.current_dir(&self.path)
                    .args(args)
                    .stdin(Stdio::from(file)),
                SandboxCommandOutput::Stderr,
                &description,
            )
            .with_context(|| format!("In working tree {}", self.path.display()))
            .context("git sparse-checkout set failed")?;
        }

        // Run checkout
        // TODO: Detect whether we need to do this or parameterize whether it happens.
        if first_run {
            let span = info_span!("Checking out initial tree", path = ?self.path);
            let _guard = span.enter();

            let description = format!("Running git checkout in {}", self.path.display());
            let (mut cmd, scmd) = git_helper::git_command(description.clone(), app)?;
            scmd.ensure_success_or_log(
                cmd.current_dir(&self.path).arg("checkout"),
                SandboxCommandOutput::Stderr,
                &description,
            )
            .context("git checkout failed")?;
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

    pub fn default_working_tree_patterns(&self) -> PatternSet {
        SOURCE_BASELINE_PATTERNS.clone()
    }

    fn apply_working_tree_patterns(&self, app: Arc<App>) -> Result<()> {
        let patterns = self.default_working_tree_patterns();
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
        let mut pattern_set = PatternSet::new();
        pattern_set.extend(BUILD_FILE_PATTERNS.clone());
        pattern_set.extend(SOURCE_BASELINE_PATTERNS.clone());
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
                        patterns.insert(Pattern::RecursiveDirectory {
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
        let outlining_tree = if outlining_tree_path.is_dir() {
            Some(OutliningTree::new(Arc::new(WorkingTree::new(
                outlining_tree_path,
                git_dir.clone(),
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

    pub fn sync(&self, coordinates: &CoordinateSet, app: Arc<App>) -> Result<()> {
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
                debug!("Initial patterns: {:?}", &outline_patterns);
                outline_patterns.extend(SOURCE_BASELINE_PATTERNS.clone());
                debug!("Patterns + Source Baseline: {:?}", &outline_patterns);
                outline_patterns.retain_relevant();
                debug!("Filtered Patterns: {:?}", &outline_patterns);

                working_tree
                    .apply_sparse_patterns(&outline_patterns, true, app)
                    .context("Failed to apply outlined patterns to working tree")?;
            }
            _ => {
                bail!("Synchronization is only possible in a repo with both working and outlining trees");
            }
        }

        Ok(())
    }

    pub fn create_outlining_tree(&self) -> Result<()> {
        let path = Self::outlining_tree_path(&self.git_dir);

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

        let work_tree_gitdir = self.git_dir.join("worktrees").join(OUTLINING_TREE_NAME);
        // Apply the correct sparse patterns
        let outlining_tree = OutliningTree::new(Arc::new(WorkingTree::new(path, work_tree_gitdir)));
        outlining_tree.apply_build_file_patterns(self.app.clone())
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
}
