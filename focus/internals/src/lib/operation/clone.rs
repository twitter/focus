use crate::{
    app::App,
    coordinate::CoordinateSet,
    model::{
        layering::{Layer, LayerSet, LayerSets, RichLayerSet},
        repo::Repo,
    },
    tracker::Tracker,
    util::{self, git_helper, sandbox_command::SandboxCommandOutput},
};
use anyhow::{bail, Context, Result};
use chrono::{Duration, Utc};
use git2::Repository;
use tracing::{debug, info, warn};
use url::Url;

use std::{
    ffi::OsString,
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug)]
pub enum Origin {
    /// Clone from a local path
    Local(PathBuf),

    /// Clone from a remote URL
    Remote(Url),
}

impl TryFrom<&str> for Origin {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if let Ok(url) = Url::parse(value) {
            Ok(Origin::Remote(url))
        } else {
            let dense_repo_path = PathBuf::from(value);
            let dense_repo_path = util::paths::expand_tilde(dense_repo_path.as_path())?;
            Ok(Origin::Local(dense_repo_path))
        }
    }
}

/// Entrypoint for clone operations.
pub fn run(
    origin: Origin,
    sparse_repo_path: PathBuf,
    branch: String,
    coordinates: Vec<String>,
    layers: Vec<String>,
    copy_branches: bool,
    days_of_history: u64,
    app: Arc<App>,
) -> Result<()> {
    match origin {
        Origin::Local(dense_repo_path) => clone_local(
            &dense_repo_path,
            &sparse_repo_path,
            branch,
            copy_branches,
            days_of_history,
            app.clone(),
        ),
        Origin::Remote(url) => {
            clone_remote(url, &sparse_repo_path, branch, days_of_history, app.clone())
        }
    }?;

    set_up_sparse_repo(&sparse_repo_path, layers, coordinates, app)
}

// TODO: Push configuration?

/// Clone from a local path on disk.
fn clone_local(
    dense_repo_path: &Path,
    sparse_repo_path: &Path,
    branch: String,
    copy_branches: bool,
    days_of_history: u64,
    app: Arc<App>,
) -> Result<()> {
    info!("Dense repo path: {}", dense_repo_path.display());
    let dense_repo_path = if dense_repo_path.is_absolute() {
        dense_repo_path.to_owned()
    } else {
        std::env::current_dir()
            .expect("Failed determining current directory")
            .join(dense_repo_path)
    };

    if !dense_repo_path.is_absolute() {
        bail!("Dense repo path must be absolute");
    }

    if sparse_repo_path.is_dir() {
        bail!("Sparse repo directory already exists");
    }

    let ui = app.ui();

    enable_filtering(&dense_repo_path)
        .context("setting configuration options in the dense repo")?;

    let url = Url::from_file_path(&dense_repo_path)
        .expect("Failed to convert dense repo path to a file URL");

    ui.status(format!(
        "Cloning {} -> {}",
        &dense_repo_path.display(),
        &sparse_repo_path.display()
    ));
    clone_shallow(
        &url,
        sparse_repo_path,
        &branch,
        copy_branches,
        days_of_history,
        app.clone(),
    )
    .context("Failed to clone the repository")?;

    let dense_repo = Repository::open(&dense_repo_path).context("Opening dense repo")?;
    let sparse_repo = Repository::open(&sparse_repo_path).context("Opening sparse repo")?;

    ui.log("Clone", "Configuring remotes");
    set_up_remotes(&dense_repo, &sparse_repo).context("Failed to set up the remotes")?;

    // Set fetchspec for primary branch
    {
        let fetch_spec = format!("refs/heads/{}:refs/remotes/origin/{}", branch, branch);
        sparse_repo
            .remote_add_fetch("origin", &fetch_spec)
            .context("Failed add fetchspec for branch")?;
    }

    if copy_branches {
        ui.log("Clone", "Copying branches from dense repo");
        info!("Copying branches");
        copy_local_branches(&dense_repo, &sparse_repo, &branch, app)
            .context("Failed to copy references")?;
    }

    Ok(())
}

/// Enable filtering in the dense repo
fn enable_filtering(dense_repo_path: &Path) -> Result<()> {
    let description = format!(
        "Setting options in dense repository {}",
        dense_repo_path.display()
    );
    let repo = Repository::open(dense_repo_path)
        .context(description.clone())
        .context("Opening repository")?;
    let mut config = repo
        .config()
        .context(description.clone())
        .context("Reading configuration")?;
    config
        .set_bool("uploadPack.allowFilter", true)
        .context(description)
        .context("Writing configuration")?;
    Ok(())
}

fn clone_remote(
    dense_repo_url: Url,
    sparse_repo_path: &Path,
    branch: String,
    days_of_history: u64,
    app: Arc<App>,
) -> Result<()> {
    if sparse_repo_path.is_dir() {
        bail!("Sparse repo directory already exists");
    }

    let ui = app.ui();

    ui.status(format!(
        "Cloning {} -> {}",
        &dense_repo_url,
        &sparse_repo_path.display()
    ));

    // Clone the repository.
    clone_shallow(
        &dense_repo_url,
        sparse_repo_path,
        &branch,
        false,
        days_of_history,
        app,
    )
    .context("Failed to clone the repository")
}

fn set_up_sparse_repo(
    sparse_repo_path: &Path,
    layers: Vec<String>,
    coordinates: Vec<String>,
    app: Arc<App>,
) -> Result<()> {
    let ui = app.ui();

    {
        let repo = Repo::open(sparse_repo_path, app.clone()).context("Failed to open repo")?;
        // TODO: Parallelize these tree set up processes.
        ui.log("Sparse Repo", "Setting up outlining tree");
        repo.create_outlining_tree()
            .context("Failed to create the outlining tree")?;

        ui.log("Sparse Repo", "Setting up the working tree");
        repo.create_working_tree()
            .context("Failed to create the working tree")?;
    }

    // N.B. we must re-open the repo because otherwise it has no trees...
    let repo = Repo::open(sparse_repo_path, app.clone()).context("Failed to open repo")?;
    let coordinate_set = {
        let outlining_tree = repo.outlining_tree().expect("No outlining tree");
        let outlining_tree_underlying = outlining_tree.underlying();
        let working_tree = repo.working_tree().expect("No working tree");

        compute_and_store_initial_selection(
            outlining_tree_underlying.path(),
            working_tree.path(),
            layers,
            coordinates,
            app.clone(),
        )?
    };

    repo.sync(&coordinate_set, app.clone())
        .context("Sync failed")?;

    repo.working_tree().unwrap().write_sync_point_ref()?;

    set_up_bazel_preflight_script(sparse_repo_path)?;

    Tracker::default()
        .ensure_registered(sparse_repo_path, app)
        .context("adding sparse repo to the list of tracked repos")?;

    Ok(())
}

pub(crate) fn named_layers_from_repo(repo: &Path, layer_names: &[String]) -> Result<LayerSet> {
    let layer_sets = LayerSets::new(repo);
    let rich_layer_set = RichLayerSet::new(
        layer_sets
            .available_layers()
            .context("getting available layers")?,
    )?;

    let mut layers = Vec::<Layer>::new();
    for layer_name in layer_names {
        if let Some(layer) = rich_layer_set.get(layer_name) {
            layers.push(layer.clone());
        } else {
            bail!("Layer named '{}' not found", &layer_name)
        }
    }

    Ok(LayerSet::new(layers.into_iter().collect()))
}

fn compute_and_store_initial_selection(
    outlining_tree_path: &Path,
    working_tree_path: &Path,
    layers: Vec<String>,
    coordinates: Vec<String>,
    app: Arc<App>,
) -> Result<CoordinateSet> {
    let working_tree_layers = LayerSets::new(working_tree_path);

    let mut layer_set = working_tree_layers
        .mandatory_layers()
        .context("Failed to resolve mandatory layers")?;

    let adhoc_set = LayerSet::new(vec![Layer::new(
        "adhoc",
        "Ad-hoc layer",
        false,
        coordinates,
    )]);

    let layers_from_outlining_tree = named_layers_from_repo(outlining_tree_path, &layers)
        .context("resolving user-selected layers")?;
    layer_set.extend(adhoc_set.clone());
    layer_set.extend(layers_from_outlining_tree.clone());
    layer_set.validate().context("Failed to merged layer set")?;
    let coordinates: Vec<String> = layer_set
        .layers()
        .iter()
        .flat_map(|layer| layer.coordinates())
        .cloned()
        .collect();

    // Write the ad-hoc set into the working tree
    app.ui().log("Clone", "writing ad-hoc layer set");
    working_tree_layers.store_adhoc_layers(&adhoc_set)?;

    app.ui().log("Clone", "writing selected layer set");
    let selected_layer_names: Vec<String> = layers_from_outlining_tree
        .layers()
        .iter()
        .map(|layer| layer.name().to_owned())
        .collect();
    working_tree_layers
        .push_as_selection(selected_layer_names)
        .context("Failed to write the selected layer set to the sparse repo")?;

    let coordinate_set =
        CoordinateSet::try_from(coordinates.as_slice()).context("Failed to parse coordinates")?;

    Ok(coordinate_set)
}

fn clone_shallow(
    source_url: &Url,
    destination_path: &Path,
    branch: &str,
    copy_branches: bool,
    days_of_history: u64,
    app: Arc<App>,
) -> Result<()> {
    // Unfortunately time::duration is signed
    let days_of_history: i64 = days_of_history.try_into()?;

    let sparse_repo_dir_parent = destination_path
        .parent()
        .context("Failed to determine sparse repo parent directory")?;

    let description = format!("Cloning {} to {}", source_url, destination_path.display());

    let shallow_since_datestamp = {
        let today = Utc::now().date();
        today
            .checked_sub_signed(Duration::days(days_of_history))
            .expect("Could not determine date 90 days ago")
            .format("%Y-%m-%d")
            .to_string()
    };

    // TODO: Reconsider single-branch
    let (mut cmd, scmd) = git_helper::git_command(description, app)?;
    let mut args: Vec<OsString> = vec![
        "clone".into(),
        "--no-checkout".into(),
        "--no-tags".into(),
        "-b".into(),
        branch.into(),
    ];
    args.push(format!("--shallow-since={}", shallow_since_datestamp).into());
    if !copy_branches {
        args.push("--single-branch".into());
    }
    args.push(source_url.as_str().into());
    args.push(destination_path.into());
    scmd.ensure_success_or_log(
        cmd.current_dir(sparse_repo_dir_parent).args(args),
        SandboxCommandOutput::Stderr,
        "clone",
    )
    .map(|_| ())
    .context("git clone failed")
}

fn set_up_remotes(dense_repo: &Repository, sparse_repo: &Repository) -> Result<()> {
    let remotes = dense_repo
        .remotes()
        .context("Failed to read remotes from dense repo")?;

    for remote_name in remotes.iter() {
        let remote_name = match remote_name {
            Some(name) => name,
            None => continue,
        };

        let dense_remote = dense_repo.find_remote(remote_name)?;
        let url = if let Some(url) = dense_remote.url() {
            url
        } else {
            bail!("Dense remote '{}' has no URL", remote_name);
        };

        let push_url = dense_remote.pushurl().unwrap_or(url);
        info!(
            "remote: {} fetch-url:{} push-url:{}",
            remote_name, url, push_url
        );

        let mut fetch_url = Url::parse(url).with_context(|| {
            format!(
                "Failed to parse the URL ('{}') for remote {}",
                url, remote_name
            )
        })?;
        let push_url = Url::parse(push_url).with_context(|| {
            format!(
                "Failed to parse the push URL ('{}') for remote {}",
                fetch_url, remote_name
            )
        })?;

        if let Some(host) = fetch_url.host() {
            // Apply Twitter-specific remote treatment.
            if host.to_string().eq_ignore_ascii_case("git.twitter.biz") {
                // If the path for the fetch URL does not begin with '/ro', add that prefix.
                if !fetch_url.path().starts_with("/ro") {
                    fetch_url.set_path(&format!("/ro{}", fetch_url.path()));
                }
            }
        } else {
            bail!("Fetch URL for remote '{}' has no host");
        }

        // Delete any existing remote with the same name in the sparse repo.
        sparse_repo.remote_delete(remote_name)?;

        // Add the remote to the sparse repo
        // sparse_repo.remote(remote_name, ur)
        info!(
            "Setting up remote {} fetch:{} push:{}",
            remote_name,
            fetch_url.as_str(),
            push_url.as_str()
        );
        sparse_repo
            .remote(remote_name, fetch_url.as_str())
            .with_context(|| {
                format!(
                    "Configuring fetch URL remote {} in the sparse repo failed",
                    &remote_name
                )
            })?;
        sparse_repo
            .remote_set_pushurl(remote_name, Some(push_url.as_str()))
            .with_context(|| {
                format!(
                    "Configuring push URL for remote {} in the sparse repo failed",
                    &remote_name
                )
            })?;
    }

    Ok(())
}

fn copy_local_branches(
    dense_repo: &Repository,
    sparse_repo: &Repository,
    branch: &str,
    app: Arc<App>,
) -> Result<()> {
    let cloned_app = app;
    let ui = cloned_app.ui();

    let branches = dense_repo
        .branches(Some(git2::BranchType::Local))
        .context("Failed to enumerate local branches in the dense repo")?;

    for b in branches {
        let (b, _branch_type) = b?;
        let name = match b.name()? {
            Some(name) => name,
            None => {
                warn!(
                    "Skipping branch {:?} because it is not representible as UTF-8",
                    b.name_bytes()
                );
                continue;
            }
        };
        if name == branch {
            // Skip the primary branch since it should already be configured.
            continue;
        }

        debug!("Examining dense repo branch {}", name);
        let dense_commit = b
            .get()
            .peel_to_commit()
            .context("Failed to peel branch ref to commit")?;

        match sparse_repo.find_commit(dense_commit.id()) {
            Ok(sparse_commit) => match sparse_repo.branch(name, &sparse_commit, false) {
                Ok(_new_branch) => {
                    ui.log(
                        "Branch Copy",
                        format!("Created branch {} ({})", name, sparse_commit.id()),
                    );
                }
                Err(e) => {
                    ui.log(
                        "Branch Copy",
                        format!("Could not create branch {} in the sparse repo: {}", name, e),
                    );
                }
            },
            Err(_) => {
                ui.log("Branch Copy",
                format!("Could not create branch {} in the sparse repo because the commit ({}) does not exist!",
                name, dense_commit.id()));
            }
        }
    }

    Ok(())
}

// Set git config key focus.sync-point to HEAD
fn set_up_bazel_preflight_script(sparse_repo: &Path) -> Result<()> {
    use std::io::prelude::*;
    use std::os::unix::prelude::PermissionsExt;

    let sparse_focus_dir = sparse_repo.join(".focus");
    if !sparse_focus_dir.is_dir() {
        std::fs::create_dir(sparse_focus_dir.as_path()).with_context(|| {
            format!("failed to create directory {}", sparse_focus_dir.display())
        })?;
    }
    let preflight_script_path = sparse_focus_dir.join("preflight");
    {
        let mut preflight_script_file = BufWriter::new(
            File::create(preflight_script_path.as_path())
                .context("writing the build preflight script")?,
        );

        writeln!(preflight_script_file, "#!/bin/sh")?;
        writeln!(preflight_script_file)?;
        writeln!(
            preflight_script_file,
            "RUST_LOG=error exec focus detect-build-graph-changes"
        )?;
    }

    let mut perms = std::fs::metadata(preflight_script_path.as_path())
        .context("Reading permissions of the preflight script failed")?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(preflight_script_path, perms)
        .context("Setting permissions of the preflight script failed")?;

    Ok(())
}

#[cfg(test)]
mod test {
    use std::{
        collections::HashSet,
        path::{Path, PathBuf},
        process::Command,
        sync::Arc,
    };

    use anyhow::{bail, Context, Result};
    use git2::Repository;
    use tracing::info;

    use crate::operation::testing::integration::RepoPairFixture;
    use crate::{app::App, model::repo::Repo, testing::init_logging};

    static MAIN_BRANCH_NAME: &str = "master";

    #[test]
    fn clone_contains_an_initial_layer_set() -> Result<()> {
        init_logging();

        let mut fixture = RepoPairFixture::new()?;
        let library_a_coord = String::from("bazel://library_a/...");
        fixture.coordinates.push(library_a_coord.clone());
        let project_b_layer_label = String::from("team_zissou/project_b");
        fixture.layers.push(project_b_layer_label.clone());

        fixture.perform_clone()?;

        let sparse_repo = fixture.sparse_repo()?;
        let working_tree = sparse_repo.working_tree().unwrap();
        let layer_sets = working_tree.layer_sets()?;

        {
            let ad_hoc_layers = layer_sets.adhoc_layers().unwrap().unwrap();
            let layers = ad_hoc_layers.layers();
            insta::assert_debug_snapshot!(layers, @r###"
            [
                Layer {
                    name: "adhoc",
                    description: "Ad-hoc layer",
                    mandatory: false,
                    coordinates: [
                        "bazel://library_a/...",
                    ],
                },
            ]
            "###);
        }
        {
            let selected_layers = layer_sets
                .selected_layers()
                .unwrap()
                .expect("Should have had some layers");
            let layers = selected_layers.layers();
            insta::assert_debug_snapshot!(layers, @r###"
            [
                Layer {
                    name: "team_zissou/project_b",
                    description: "Stuff relating to project B",
                    mandatory: false,
                    coordinates: [
                        "bazel://project_b/...",
                    ],
                },
            ]
            "###);
        }

        Ok(())
    }

    #[test]
    fn local_clone_smoke_test() -> Result<()> {
        init_logging();
        let fixture = RepoPairFixture::new()?;

        // Set up a remote that mimicks source so that we can check that the setting of fetch and push URLs.
        Command::new("git")
            .arg("remote")
            .arg("add")
            .arg("origin")
            .arg("https://git.twitter.biz/focus-test-repo")
            .current_dir(&fixture.dense_repo_path)
            .status()
            .expect("git remote set-url failed");

        // Make another branch
        Command::new("git")
            .arg("switch")
            .arg("-c")
            .arg("branch_two")
            .current_dir(&fixture.dense_repo_path)
            .status()
            .expect("git switch failed");

        let app = Arc::new(App::new(false, false)?);

        fixture.perform_clone()?;

        let git_repo = Repository::open(&fixture.sparse_repo_path)?;
        assert!(!git_repo.is_bare());

        // Check the remote URLs
        let origin_remote = git_repo.find_remote("origin")?;
        assert_eq!(
            origin_remote.url().unwrap(),
            "https://git.twitter.biz/ro/focus-test-repo"
        );
        assert_eq!(
            origin_remote.pushurl().unwrap(),
            "https://git.twitter.biz/focus-test-repo"
        );

        // Check branches
        let main_branch = git_repo
            .find_branch(MAIN_BRANCH_NAME, git2::BranchType::Local)
            .context("Failed to find main branch")?;
        let main_branch_commit_id = main_branch.get().peel_to_commit()?.id();

        for possible_branch in git_repo.branches(None)? {
            match possible_branch {
                Ok((branch, kind)) => {
                    info!("{:?} branch: {}", kind, branch.name().unwrap().unwrap());
                }
                Err(e) => {
                    bail!("Error enumerating local branches: {}", e);
                }
            }
        }

        git_repo
            .find_branch("branch_two", git2::BranchType::Local)
            .context("Failed to find branch_two")?;

        // TODO: Test refspecs from remote config
        let model_repo = Repo::open(&fixture.sparse_repo_path, app)?;

        // Check sync point
        let sync_point_oid = model_repo
            .working_tree()
            .unwrap()
            .read_sync_point_ref()?
            .unwrap();
        assert_eq!(sync_point_oid, main_branch_commit_id);

        // Check tree contents
        {
            let outlining_tree = model_repo.outlining_tree().unwrap();
            let outlining_tree_underlying = outlining_tree.underlying();
            let outlining_tree_path = outlining_tree_underlying.path();
            let walker = walkdir::WalkDir::new(outlining_tree_path).follow_links(false);

            let outlining_tree_paths: HashSet<PathBuf> = walker
                .into_iter()
                .map(|m| {
                    m.unwrap()
                        .path()
                        .strip_prefix(&outlining_tree_path)
                        .unwrap()
                        .to_owned()
                })
                .collect();

            assert!(outlining_tree_paths.contains(Path::new("focus")));
            assert!(outlining_tree_paths.contains(Path::new("WORKSPACE")));
            assert!(outlining_tree_paths.contains(Path::new("library_a/BUILD")));
            assert!(outlining_tree_paths.contains(Path::new("library_b/BUILD")));
            assert!(outlining_tree_paths.contains(Path::new(
                "project_a/src/main/java/com/example/cmdline/BUILD"
            )));
            assert!(outlining_tree_paths.contains(Path::new(
                "project_b/src/main/java/com/example/cmdline/BUILD"
            )));
            assert!(outlining_tree_paths.contains(Path::new("mandatory_z/BUILD")));
        }

        {
            let working_tree = model_repo.working_tree().unwrap();
            let working_tree_path = working_tree.path();
            let walker = walkdir::WalkDir::new(working_tree_path).follow_links(false);
            let working_tree_paths: HashSet<PathBuf> = walker
                .into_iter()
                .map(|m| {
                    m.unwrap()
                        .path()
                        .strip_prefix(&working_tree_path)
                        .unwrap()
                        .to_owned()
                })
                .collect();

            // N.B. Only the mandatory layer is checked out
            assert!(working_tree_paths.contains(Path::new("focus")));
            assert!(working_tree_paths.contains(Path::new("mandatory_z")));
            assert!(working_tree_paths.contains(Path::new("mandatory_z/BUILD")));
            assert!(working_tree_paths.contains(Path::new("mandatory_z/quotes.txt")));

            assert!(!working_tree_paths.contains(Path::new("library_a/BUILD")));
            assert!(!working_tree_paths.contains(Path::new("library_b/BUILD")));
            assert!(!working_tree_paths.contains(Path::new("library_a/BUILD")));
            assert!(!working_tree_paths.contains(Path::new("library_a/BUILD")));
            assert!(!working_tree_paths.contains(Path::new(
                "project_a/src/main/java/com/example/cmdline/BUILD"
            )));
            assert!(!working_tree_paths.contains(Path::new(
                "project_b/src/main/java/com/example/cmdline/BUILD"
            )));
        }

        Ok(())
    }
}
