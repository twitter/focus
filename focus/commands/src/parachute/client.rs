use anyhow::{bail, Context, Result};
use focus_formats::analysis::{Artifact, PathFragment};
use std::path::Path;
use std::{
    cell::Cell,
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    env::current_dir,
    iter::FromIterator,
    path::PathBuf,
};
use std::fs::File;
use std::io::prelude::*;
use std::thread;
use std::ffi::OsString;

use crate::main;

const SPARSE_PROFILE_PRELUDE: &str = "/tools/\n/pants-plugins/\n/pants-support/\n/3rdparty/\n";

fn exhibit_file(file: &Path, title: &str) -> Result<()> {
    use std::fs::File;
    use std::io::{self, BufRead};
    use std::path::Path;

    let file = File::open(file)?;
    let lines = io::BufReader::new(file).lines();
    log::info!("--- Begin {} ---", title);
    for line in lines {
        if let Ok(line) = line {
            log::info!("{}", line);
        }
    }
    log::info!("--- End {} ---", title);

    Ok(())
}

fn git_binary() -> Result<OsString> {
    Ok(OsString::from("/Users/wilhelm/opt/git/bin/git"))
}

fn add_implicit_coordinates(v: &mut Vec<String>) {
    let implicit_coordinates: Vec<String> = vec![
        String::from("//tools/implicit_deps:thrift-implicit-deps-impl"),
        String::from("//scrooge-internal/..."),
        String::from("//loglens/loglens-logging/..."),
    ];
    v.extend(implicit_coordinates)
}

pub fn create_sparse_clone(dense_repo: &PathBuf, sparse_repo: &PathBuf, coordinates: &Vec<String>, branch: &String) -> Result<()> {
    let temp_dir = tempfile::Builder::new().prefix("focus-parachute-work").tempdir().context("creating a temporary directory")?;
    let temp_dir_path = temp_dir.path();

    let sparse_profile_output = temp_dir_path.join("sparse-checkout");
    let mut coordinates = coordinates.clone();
    add_implicit_coordinates(&mut coordinates);


    let profile_generation_thread = {
        let cloned_temp_dir_path = temp_dir_path.to_owned();
        let cloned_dense_repo = dense_repo.to_owned();
        let cloned_sparse_profile_output = sparse_profile_output.to_owned();
        let cloned_coordinates = coordinates.to_vec().clone();

        thread::spawn(move || {
            log::info!("generating sparse profile");
            generate_sparse_profile(&cloned_dense_repo, &cloned_sparse_profile_output, &cloned_coordinates)
                .expect("failed to generate a sparse profile");
            log::info!("finished generating sparse profile");
        })
    };

    let clone_thread = {
        let cloned_temp_dir_path = temp_dir_path.to_owned();
        let cloned_dense_repo = dense_repo.to_owned();
        let cloned_sparse_repo = sparse_repo.to_owned();
        let cloned_branch = branch.clone();

        thread::spawn(move || {
            log::info!("creating a template clone");
            create_empty_sparse_clone(&cloned_temp_dir_path,
                                      &cloned_dense_repo,
                                      &cloned_sparse_repo,
                                      &cloned_branch)
                .expect("failed to create an empty sparse clone");
            log::info!("finished creating a template clone");
        })
    };

    profile_generation_thread.join().expect("sparse profile generation thread exited abnormally");
    clone_thread.join().expect("clone thread exited abnormally");

    {
        let cloned_temp_dir_path = temp_dir_path.to_owned();
        let cloned_dense_repo = dense_repo.to_owned();
        let cloned_sparse_repo = sparse_repo.to_owned();
        let cloned_sparse_profile_output = sparse_profile_output.clone();
        let cloned_branch = branch.clone();
        log::info!("configuring the sparse copy");
        set_sparse_checkout(&cloned_temp_dir_path, sparse_repo, &sparse_profile_output).context("setting up sparse checkout options")?;
        log::info!("checking out the working copy");
        switch_branches(&cloned_temp_dir_path, &cloned_dense_repo, &cloned_sparse_repo, &cloned_branch).context("switching branches")?;
    }
    Ok(())
}

pub fn set_sparse_checkout(temp_dir: &PathBuf, sparse_repo: &PathBuf, sparse_profile: &PathBuf) -> Result<()> {
    use std::process::{Command, Stdio};
    use std::os::unix::ffi::OsStrExt;
    {
        let git_out_path = &temp_dir.join("git-sparse-checkout-init.stdout");
        let git_out_file = File::create(&git_out_path).context("opening stdout destination file for git command")?;
        let git_err_path = &temp_dir.join("git-sparse-checkout-init.stderr");
        let git_err_file = File::create(&git_err_path).context("opening stderr destination file for git command")?;

        let output = Command::new(git_binary()?)
            .arg("sparse-checkout")
            .arg("init")
            .arg("--cone")
            .arg("--no-sparse-index") // TODO: The sparse index is somehow slower. Figure it out.
            .current_dir(&sparse_repo)
            .stdout(Stdio::from(git_out_file))
            .stderr(Stdio::from(git_err_file))
            .spawn()
            .context("spawning git sparse-checkout-init")?
            .wait_with_output()
            .context("awaiting git sparse-checkout-init")?;
        if !output.status.success() {
            exhibit_file(&git_out_path, "git sparse-checkout-init stdout")?;
            exhibit_file(&git_err_path, "git sparse-checkout-init stderr")?;
            bail!("git sparse-checkout-init failed");
        }
    }

    // TODO: Get cone checkouts working. Currently they complain about duplicate entries.
    {
        let git_out_path = &temp_dir.join("git-sparse-checkout-set.stdout");
        let git_out_file = File::create(&git_out_path).context("opening stdout destination file for git command")?;
        let git_err_path = &temp_dir.join("git-sparse-checkout-set.stderr");
        let git_err_file = File::create(&git_err_path).context("opening stderr destination file for git command")?;
        // exhibit_file(&sparse_profile, "spasre profile file");
        let sparse_profile_file = File::open(&sparse_profile).context("opening sparse profile for git command")?;

        let output = Command::new(git_binary()?)
            .arg("sparse-checkout")
            .arg("set")
            .arg("--stdin")
            .current_dir(&sparse_repo)
            .stdin(Stdio::from(sparse_profile_file))
            .stdout(Stdio::from(git_out_file))
            .stderr(Stdio::from(git_err_file))
            .spawn()
            .context("spawning git sparse-checkout-set")?
            .wait_with_output()
            .context("awaiting git sparse-checkout-set")?;

        if !output.status.success() {
            exhibit_file(&git_out_path, "git sparse-checkout-set stdout")?;
            exhibit_file(&git_err_path, "git sparse-checkout-set stderr")?;
            bail!("git sparse-checkout-set failed");
        }
    }

    Ok(())
}

pub fn switch_branches(temp_dir: &PathBuf, dense_repo: &PathBuf, sparse_repo: &PathBuf, branch: &String) -> Result<()> {
    use std::process::{Command, Stdio};
    use std::os::unix::ffi::OsStrExt;

    let git_out_path = &temp_dir.join("git-switch.stdout");
    let git_out_file = File::create(&git_out_path).context("opening stdout destination file for git command")?;
    let git_err_path = &temp_dir.join("git-switch.stderr");
    let git_err_file = File::create(&git_err_path).context("opening stderr destination file for git command")?;
    log::info!("checking out in '{}'", &sparse_repo.display());
    let output = Command::new(git_binary()?)
        .arg("checkout")
        .arg(branch)
        .current_dir(&sparse_repo)
        .stdout(Stdio::from(git_out_file))
        // .stderr(Stdio::from(git_err_file))
        .spawn()
        .context("spawning git switch")?
        .wait_with_output()
        .context("awaiting git switch")?;

    if !output.status.success() {
        exhibit_file(&git_out_path, "git switch stdout")?;
        exhibit_file(&git_err_path, "git switch stderr")?;
        bail!("git switch failed");
    }

    Ok(())
}

pub fn create_empty_sparse_clone(temp_dir: &PathBuf, dense_repo: &PathBuf, sparse_repo: &PathBuf, branch: &String) -> Result<()> {
    use std::process::{Command, Stdio};
    use std::os::unix::ffi::OsStrExt;

    let filtering_enabled = true; // If filtering is enabled, use URL-based local clone and disable --shared.
    let mut dense_url = OsString::from(if filtering_enabled { "file://" } else { "" });
    dense_url.push(dense_repo);

    let sparse_repo_dir_parent = &sparse_repo.parent().context("sparse repo parent directory does not exist")?;

    let git_out_path = &temp_dir.join("git-clone.stdout");
    let git_out_file = File::create(&git_out_path).context("opening stdout destination file for git command")?;
    // let git_err_path = &temp_dir.join("git-clone.stderr");
    // let git_err_file = File::create(&git_err_path).context("opening stderr destination file for git command")?;
    // TODO: Support --filter=sparse:oid=<blob-ish>
    log::info!("Cloning {:?} into {:?}", &dense_url, &sparse_repo);
    let output = Command::new(git_binary()?)
        .arg("clone")
        .arg("-c")
        .arg("core.compression=1")
        .arg("--sparse")
        .arg("--no-checkout")
        .arg("--no-tags")
        .arg("--single-branch")
        .arg("--depth")
        .arg("64")
        .arg("-b")
        .arg("master")
        .arg(if filtering_enabled { "--filter=blob:none" } else { "--shared" }) // Server doesn't support filtering
        .arg(dense_url)
        .arg(sparse_repo)
        .current_dir(sparse_repo_dir_parent)
        .stdout(Stdio::from(git_out_file))
        // .stderr(Stdio::from(git_err_file)) // Disable stderr redirection temporarily to exhibit status
        .spawn()
        .context("spawning git clone")?
        .wait_with_output()
        .context("awaiting git clone")?;

    if !output.status.success() {
        exhibit_file(&git_out_path, "git clone stdout")?;
        // exhibit_file(&git_err_path, "git clone stderr")?;
        bail!("git clone failed");
    }

    Ok(())
}

pub fn generate_sparse_profile(dense_repo: &Path, sparse_profile_output: &Path, coordinates: &Vec<String>) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let client = BazelRepo::new(dense_repo, coordinates.clone())?;
    let mut source_paths = HashSet::<String>::new();
    for coordinate in coordinates {
        // Do Bazel aquery for each coordinate
        let sources = client
            .involved_sources_aquery(&coordinate)
            .with_context(|| format!("determining involved sources for {}", coordinate))?;
        log::info!("{}: {} source files", coordinate, &sources.len());
        source_paths.extend(sources);
    }

    let repo_component_count = dense_repo.components().count();

    let mut aquery_dirs = client
        .involved_directories_for_sources(source_paths.iter())
        .context("determining involved directories for sources")?;

    let mut reduced_dirs = reduce_to_shortest_common_prefix(&aquery_dirs)
        .context("reducing paths to shortest common prefix (second pass)")?;

    let mut query_dirs = BTreeSet::<PathBuf>::new();
    query_dirs.extend(reduced_dirs);

    for coordinate in coordinates {
        // Do Bazel query by package for each coordinate
        let directories_for_coordinate = client
            .involved_directories_query(&coordinate)
            .with_context(|| format!("determining involved directories for {}", coordinate))?;
        log::info!("{}: {} directories", coordinate, &directories_for_coordinate.len());
        for dir in directories_for_coordinate {
            let absolute_path = dense_repo.join(dir);
            query_dirs.insert(absolute_path);
        }
    }

    let mut f = File::create(&sparse_profile_output).context("creating output file")?;
    f.write_all(&SPARSE_PROFILE_PRELUDE.as_bytes()).context("writing sparse profile prelude")?;
    for dir in &query_dirs {
        let mut line = Vec::<u8>::new();
        line.extend(b"/"); // Paths have a '/' prefix
        {
            let mut relative_path = PathBuf::new();
            // aquery returns explicit paths. relativize them.
            for component in dir.components().skip(repo_component_count) {
                relative_path.push(component);
            }

            line.extend(relative_path.as_os_str().as_bytes());
        }
        line.extend(b"/\n"); // Paths have a '/' suffix
        f.write_all(&line[..]).with_context(|| { format!("writing output (item={:?})", dir) })?;

    }
    f.sync_data().context("syncing data")?;
    // log::info!("Reduced {} coordinate file sets to {} directories", &coordinates.len(), &reduced_dirs.len());

    Ok(())
}

struct BazelRepo {
    dense_repo: PathBuf,
    coordinates: Vec<String>,
}


impl BazelRepo {
    pub fn new(dense_repo: &Path, coordinates: Vec<String>) -> Result<Self> {
        Ok(Self {
            dense_repo: dense_repo.to_owned(),
            coordinates: coordinates,
        })
    }

    fn find_closest_directory_with_build_file(
        &self,
        file: &Path,
        ceiling: &Path,
    ) -> Result<Option<PathBuf>> {
        let mut dir = file.parent().context("getting parent directory of file")?;
        loop {
            if dir == ceiling {
                return Ok(None);
            }

            for entry in std::fs::read_dir(&dir)
                .with_context(|| format!("reading directory contents {}", dir.display()))?
            {
                let entry = entry.context("reading directory entry")?;
                if entry.file_name() == "BUILD" {
                    // Match BUILD, BUILD.*
                    return Ok(Some(dir.to_owned()));
                }
            }

            dir = dir
                .parent()
                .context("getting parent of current directory")?;
        }
    }

    // Given a source path, get the closest directory with a BUILD file.
    pub fn involved_directories_for_sources<'a, I>(&self, sources: I) -> Result<BTreeSet<PathBuf>>
    where
        I: IntoIterator<Item = &'a String>,
        I::IntoIter: 'a,
    {
        let mut results = BTreeSet::<PathBuf>::new();
        for source in sources {
            let source_path = self.dense_repo.join(source);
            if let Some(build_dir) = self
                .find_closest_directory_with_build_file(&source_path, self.dense_repo.as_path())
                .with_context(|| format!("finding a build file for {}", source))?
            {
                results.insert(build_dir);
            } else {
                // In the case that there is no BUILD file, include the directory itself.
                let parent = source_path
                    .parent()
                    .context("getting parent directory for BUILD-less file")?
                    .to_owned();
                results.insert(parent);
            }
        }
        Ok(results)
    }

    // Use bazel query to get involved packages and turn them into directories.
    pub fn involved_directories_query(&self, coordinate: &str) -> Result<Vec<String>> {
        // N.B. `bazel aquery` cannot handle unions ;(

        use focus_formats::analysis::*;
        use prost::Message;
        use std::fs::File;
        use std::io::prelude::*;
        use std::process::{Command, Stdio};
        use tempfile::Builder;

        let temp_dir = Builder::new().prefix("focus-parachute-work").tempdir().context("creating a temporary directory")?;
        let temp_dir_path = temp_dir.path();
        let bazel_out_path = &temp_dir_path.join("bazel-query.stdout");
        let bazel_out_file = File::create(&bazel_out_path).context("opening stdout destination file for bazel command")?;
        let bazel_err_path = &temp_dir_path.join("bazel-query.stderr");
        let bazel_err_file = File::create(&bazel_err_path).context("opening stderr destination file for bazel command")?;

        let mut directories = Vec::<String>::new();

        let query = format!("deps({})", coordinate);

        // Run Bazel query
        let output = Command::new("./bazel")
            .arg("query")
            .arg(query)
            .arg("--output=package")
            .current_dir(&self.dense_repo)
            .stdout(Stdio::from(bazel_out_file))
            .stderr(Stdio::from(bazel_err_file))
            .spawn()
            .context("spawning bazel query")?
            .wait_with_output()
            .context("awaiting bazel query")?;

        if !output.status.success() {
            exhibit_file(&bazel_err_path, "bazel stderr")?;
            bail!("bazel query failed");
        }

        {
            let file = File::open(bazel_out_path)?;
            for line in std::io::BufReader::new(file).lines() {
                if let Ok(line) = line {
                    let path = PathBuf::from(&line);
                    if !&line.starts_with("@") && !path.starts_with("bazel-out/") && !path.starts_with("external/") {
                        let absolute_path = &self.dense_repo.join(&path);
                        if let Some(path) = absolute_path.to_str() {
                            directories.push(path.to_owned());
                        } else {
                            bail!("Path '{}' contains characters that cannot be safely converted", &path.display());
                        }
                    }
                }
            }
        }

        Ok(directories)
    }

    pub fn involved_sources_aquery(&self, coordinate: &str) -> Result<Vec<String>> {
        // N.B. `bazel aquery` cannot handle unions ;(

        use focus_formats::analysis::*;
        use prost::Message;
        use std::fs::File;
        use std::io::prelude::*;
        use std::process::{Command, Stdio};
        use tempfile::Builder;

        let temp_dir = Builder::new().prefix("focus-parachute-work").tempdir().context("creating a temporary directory")?;
        let temp_dir_path = temp_dir.path();
        let bazel_out_path = &temp_dir_path.join("bazel-aquery.stdout");
        let bazel_out_file = File::create(&bazel_out_path).context("opening stdout destination file for bazel command")?;
        let bazel_err_path = &temp_dir_path.join("bazel-aquery.stderr");
        let bazel_err_file = File::create(&bazel_err_path).context("opening stderr destination file for bazel command")?;

        let mut sources = Vec::<String>::new();

        let query = format!("inputs('.*', ({} union deps({})))", coordinate, coordinate);

        // Run Bazel aquery
        let output = Command::new("./bazel")
            .arg("aquery")
            .arg(query)
            .arg("--output=proto")
            .current_dir(&self.dense_repo)
            .stdout(Stdio::from(bazel_out_file))
            .stderr(Stdio::from(bazel_err_file))
            .spawn()
            .context("spawning bazel aquery")?
            .wait_with_output()
            .context("awaiting bazel aquery")?;

        if !output.status.success() {
            exhibit_file(&bazel_err_path, "bazel stderr")?;
            bail!("bazel aquery failed");
        }

        let container: ActionGraphContainer;
        {
            let mut bytes = Vec::<u8>::new();
            let mut proto_output =
                File::open(&bazel_out_path).context("opening protobuf output for reading")?;
            proto_output
                .read_to_end(&mut bytes)
                .expect("reading proto output");
            container = ActionGraphContainer::decode(&bytes[..])
                .context("decoding action graph container protobuf")?;
        }

        let mut path_fragments = HashMap::<u32, PathFragment>::new();
        for path_fragment in container.path_fragments {
            let id = path_fragment.id;
            assert!(id != 0);
            if let Some(_) = path_fragments.insert(id, path_fragment) {
                bail!("duplicate fragment inserted with id {}", &id);
            }
        }

        let mut artifacts = HashMap::<u32, Artifact>::new();
        for artifact in container.artifacts {
            let id = artifact.id;
            if let Some(_) = artifacts.insert(artifact.id, artifact) {
                bail!("duplicate artifact inserted with id {}", id);
            }
        }

        let mut depsets = HashMap::<u32, DepSetOfFiles>::new();
        for depset in container.dep_set_of_files {
            let id = depset.id;
            if let Some(_) = depsets.insert(depset.id, depset) {
                bail!("duplicate artifact inserted with id {}", id);
            }
        }

        let qualify_path_fragment = |path_fragment_id: u32| -> Result<String> {
            let mut fragments = VecDeque::<&PathFragment>::new();
            let mut fragment_id = path_fragment_id;
            assert!(fragment_id != 0);
            loop {
                if let Some(fragment) = path_fragments.get(&fragment_id) {
                    fragments.push_front(fragment);
                    if fragment.parent_id == 0 {
                        break;
                    } else {
                        fragment_id = fragment.parent_id;
                    }
                } else {
                    bail!("missing path fragment")
                }
            }

            let mut label = Vec::<&str>::new();
            for fragment in fragments {
                label.push(&fragment.label);
            }
            Ok(label.join("/"))
        };

        for action in container.actions {
            let mut path_fragment_ids = Vec::<u32>::new();

            let mut process_depset = |ids: &Vec<u32>| -> Result<()> {
                for artifact_id in ids {
                    match artifacts.get(&artifact_id) {
                        Some(artifact) => {
                            path_fragment_ids.push(artifact.path_fragment_id);
                        }
                        None => {
                            bail!("missing artifact with id {}", &artifact_id);
                        }
                    }
                }

                Ok(())
            };

            let mut transitive_depsets = HashSet::<u32>::new();
            for dep_set_id in &action.input_dep_set_ids {
                match depsets.get(&dep_set_id) {
                    Some(depset) => {
                        transitive_depsets.extend(&depset.transitive_dep_set_ids);
                        process_depset(&depset.direct_artifact_ids)
                            .context("processing direct depsets")?;
                    }
                    None => {
                        bail!("missing direct depset with id {}", &dep_set_id);
                    }
                }
            }

            // Remove directs
            for dep_set_id in &action.input_dep_set_ids {
                transitive_depsets.remove(&dep_set_id);
            }

            // Process transitive depsets
            for dep_set_id in transitive_depsets {
                let ids = vec![dep_set_id];
                process_depset(&ids).context("processing transitive depsets")?;
            }

            for path_fragment_id in path_fragment_ids {
                let path =
                    qualify_path_fragment(path_fragment_id).context("qualifying path fragment")?;
                // TODO: Factor out these forbidden prefixes.
                if !path.starts_with("bazel-out/") && !path.starts_with("external/") {
                    sources.push(path);
                }
            }
        }
        Ok(sources)
    }
}

fn reduce_to_shortest_common_prefix(paths: &BTreeSet<PathBuf>) -> Result<BTreeSet<PathBuf>> {
    let mut results = BTreeSet::<PathBuf>::new();
    let mut last_path: Option<PathBuf> = None;
    for path in paths {
        let insert = match &last_path {
            Some(last_path) => !path.starts_with(last_path),
            None => true,
        };

        if insert {
            results.insert(path.clone());
            last_path = Some(path.to_owned());
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reduce_to_shortest_common_prefix() -> Result<()> {
        let tempdir = tempfile::tempdir()?;
        let dir = tempdir.path();
        let mut set = BTreeSet::<PathBuf>::new();
        let a0 = dir.join("a0");
        let a0_b = a0.join("b");
        let a0_b_c = a0_b.join("c");
        let a1 = dir.join("a1");

        set.insert(a0_b.clone());
        set.insert(a0_b_c.clone());
        set.insert(a1.clone());

        let resulting_set = reduce_to_shortest_common_prefix(&set)?;

        assert_eq!(resulting_set.len(), 2);
        assert!(resulting_set.contains(&a0_b));
        assert!(resulting_set.contains(&a1));

        Ok(())
    }
}
