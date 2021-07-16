use std::{cell::Cell, collections::{BTreeSet, HashMap, HashSet, VecDeque}, env::current_dir, iter::FromIterator, path::PathBuf};
use anyhow::{Context, Result, bail};
use focus_formats::analysis::{Artifact, PathFragment};
use std::path::Path;

use crate::main;

pub fn run_client(source: &Path, target: &Path, coordinates: Vec<String>) -> Result<()> {
	let client = Client::new(source, target, coordinates.clone())?;
	let mut source_paths = HashSet::<String>::new();
	for coordinate in &coordinates {
		let sources = client.involved_sources(&coordinate).with_context(|| format!("determining involved sources for {}", coordinate))?;
		log::info!("{}: {} source files", coordinate, &sources.len());
		source_paths.extend(sources);
	}
	let mut dirs = client.involved_directories_for_sources(source_paths.iter()).context("determining involved directories for sources")?;
	for dir in &dirs {
		log::info!("Path: {}", dir.display());
	}

	// let source_component_count = source.components().collect::<PathBuf::components>::().len();
	let source_component_count = source.components().count();

	log::info!("---shortest-common-prefix filtered paths---");
	let reduced_dirs = reduce_to_shortest_common_prefix(&dirs).context("reducing paths to shortest common prefix")?;
	for dir in reduced_dirs {
		let mut relative_path = PathBuf::new();
		for component in dir.components().skip(source_component_count) {
			relative_path.push(component);
		}
		log::info!(" {}", relative_path.display());
	}
	Ok(())
}

struct Client {
	source: PathBuf,
	target: PathBuf, 
	coordinates: Vec<String>,
}

impl Client {
	pub fn new(source: &Path, target: &Path, coordinates: Vec<String>) -> Result<Self> {

		Ok(Self{
			source: source.to_owned(),
			target: target.to_owned(),
			coordinates: coordinates,
		})
	}

	fn find_closest_directory_with_build_file(&self, file: &Path, ceiling: &Path) -> Result<Option<PathBuf>> {
		let mut dir = file.parent().context("getting parent directory of file")?;
		loop {
			if dir == ceiling {
				return Ok(None)
			}
			
			for entry in std::fs::read_dir(&dir).with_context( || format!("reading directory contents {}", dir.display()))? {
				let entry = entry.context("reading directory entry")?;
				if entry.file_name() == "BUILD" {
					// Match BUILD, BUILD.* 
					return Ok(Some(dir.to_owned()))
				}
			}

			dir = dir.parent().context("getting parent of current directory")?;
		}
	}

	// Given a source path, get the closest directory with a BUILD file.
	pub fn involved_directories_for_sources<'a, I>(&self, sources: I) -> Result<BTreeSet<PathBuf>> 
		where I: IntoIterator<Item=&'a String>, I::IntoIter: 'a
	{
		let mut results = BTreeSet::<PathBuf>::new();
		for source in sources {
		let source_path = self.source.join(source);
			if let Some(build_dir) = self.find_closest_directory_with_build_file(&source_path, self.source.as_path()).with_context(|| { format!("finding a build file for {}", source)})? {
				results.insert(build_dir);
			} else {
				// In the case that there is no BUILD file, include the directory itself.
				let parent = source_path.parent().context("getting parent directory for BUILD-less file")?.to_owned();
				results.insert(parent);
			}
		}
		Ok(results)
	}

	pub fn involved_sources(&self, coordinate: &str) -> Result<Vec<String>> {
		// N.B. `bazel aquery` cannot handle unions ;(

		use prost::Message;
		use focus_formats::analysis::*;
		use std::io::prelude::*;
		use std::fs::File;
		use std::process::{Command, Stdio};

		let mut work_dir = tempfile::tempdir()?;
		
		let mut output_path = work_dir.path().join("analysis");
		
		let mut sources = Vec::<String>::new();

		let query = format!("inputs('.*', ({} union deps({})))", coordinate, coordinate);
	
		// Run Bazel aquery
		let output = Command::new("./bazel")
		    .arg("aquery")
		    .arg("--output=proto")
		    .arg(query)
            .current_dir(&self.source)
		    .stdout(Stdio::from(File::create(&output_path).context("opening output for write")?))
		    .spawn()
		    .context("spawning bazel aquery")?
		    .wait_with_output()
		    .context("awaiting bazel aquery")?;

		if !output.status.success() {
			bail!("bazel aquery failed");
		}

		let container: ActionGraphContainer;
		{
			let mut bytes = Vec::<u8>::new();
			let mut proto_output = File::open(&output_path).context("opening protobuf output for reading")?;
			proto_output.read_to_end(&mut bytes).expect("reading proto output");
			container = ActionGraphContainer::decode(&bytes[..]).context("decoding action graph container protobuf")?;
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
						},
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
						process_depset(&depset.direct_artifact_ids).context("processing direct depsets")?;
					},
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
				let ids = vec!(dep_set_id);
				process_depset(&ids).context("processing transitive depsets")?;
			}

			// log::info!("Action[{}] (target_id: {}, key{}) ", action.mnemonic, action.target_id, action.action_key);
			for path_fragment_id in path_fragment_ids {
				let path = qualify_path_fragment(path_fragment_id).context("qualifying path fragment")?;
				if !path.starts_with("bazel-out/") && !path.starts_with("external/") {
					// log::info!("- {}", &path);
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

struct Coord {
	repo: Option<String>,
	package: String
	
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