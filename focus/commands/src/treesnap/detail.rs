use anyhow::Result;
use env_logger::{self, Env};
use log::{debug, error, info};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use structopt::StructOpt;

use git2::{ObjectType, TreeEntry, TreeWalkMode, TreeWalkResult};
use internals::error::AppError;
use sha2::digest::DynDigest;
use sha2::{Digest, Sha224, Sha256};
use std::collections::HashSet;
use std::fs::Permissions;

fn normalize_tree_entry_path(path: &str, tree_entry: &TreeEntry) -> Result<String, AppError> {
    let mut path = path.to_string();
    if let Some(name) = tree_entry.name() {
        path.push_str(name);
        Ok(path)
    } else {
        Err(AppError::None())
    }
}

lazy_static! {
    static ref EXTENSIONS_WITH_FULL_CONTENTS_REQUIRED: HashSet<&'static str> = {
        let mut s = HashSet::new();
        s.insert("bzl");
        s
    };
    static ref FILENAMES_WITH_FULL_CONTENTS_REQUIRED: HashSet<&'static str> = {
        let mut s = HashSet::new();
        s.insert("WORKSPACE");
        s.insert("BUILD");
        s
    };
}

fn full_contents_required_predicate(name: &str) -> bool {
    let path = Path::new(name);
    if let Some(extension) = path.extension() {
        if EXTENSIONS_WITH_FULL_CONTENTS_REQUIRED.contains(extension.to_str().unwrap()) {
            return true;
        }
    }

    FILENAMES_WITH_FULL_CONTENTS_REQUIRED.contains(path.file_name().unwrap().to_str().unwrap())
}

pub fn snapshot(repo: &Path, output: &Path) -> Result<(), AppError> {
    use focus_formats::treesnap::*;

    let repo = git2::Repository::open(repo)?;
    let head_reference = repo.head()?;
    let commit = head_reference.peel_to_commit()?;
    info!("Commit {:?}", commit.id());

    let mut aborted = false;
    let mut dir_stack = Vec::<String>::new();
    let mut node_stack = Vec::<bool>::new();

    let count = commit.tree()?.walk(TreeWalkMode::PreOrder, |path, entry| {
        if let Ok(normal_path) = normalize_tree_entry_path(path, entry) {
            let permissions: Permissions = PermissionsExt::from_mode(entry.filemode() as u32);
            info!("{:?} {:?} {:?}", normal_path, entry.kind(), permissions);
            match entry.kind() {
                Some(ObjectType::Tree) => {
                    // TODO: Refactor. This works but it's ugly; there's probably some nice iterator we can use here.
                    let split_path: Vec<String> =
                        normal_path.split("/").map(|s| s.to_owned()).collect();

                    info!("* Dir change: {}", &normal_path);
                    let mut mutual_components: usize = 0;
                    for (ix, component) in split_path.iter().enumerate() {
                        if let Some(stack_component) = dir_stack.get(ix) {
                            if component == stack_component {
                                mutual_components += 1;
                            }
                        } else {
                            break;
                        }
                    }

                    let to_pop = dir_stack.len() - mutual_components;
                    for i in 0..to_pop {
                        let popped_dir = dir_stack.pop().unwrap();
                        info!("Pop {}", popped_dir);
                        node_stack.pop();
                    }

                    for new_component in split_path.iter().skip(mutual_components) {
                        dir_stack.push(new_component.clone());
                        node_stack.push(true);

                        info!("Push {}", new_component);
                    }
                    info!("* Dir stack: {:?}", dir_stack);
                }
                Some(ObjectType::Blob) => {
                    let generic_object = entry.to_object(&repo).expect("Fetching object failed");
                    let blob_object = generic_object.as_blob().unwrap();

                    let content = blob_object.content();
                    let mut digest = Sha256::new();
                    Digest::update(&mut digest, content);
                    let content_digest_output = digest.finalize();
                    let content_hash = ContentDigest {
                        algorithm: content_digest::Algorithm::Sha256 as i32,
                        value: Vec::<u8>::from(content_digest_output.as_slice()),
                    };

                    if full_contents_required_predicate(entry.name().unwrap()) {
                        info!("Full contents required!");
                        // op.set_content();
                        let blob = content::Blob {
                            content_hash: Some(content_hash),
                            content_is_inline: true,
                            content: Vec::from(content),
                        };
                    } else {
                        let blob = content::Blob {
                            content_hash: Some(content_hash),
                            content_is_inline: false,
                            content: Vec::<u8>::new(),
                        };
                    }
                }
                Some(ObjectType::Commit) => {}
                Some(ObjectType::Tag) => {}
                Some(ObjectType::Any) => {
                    panic!("Unexpected Any type");
                }
                _ => {
                    panic!("Unexpected object type")
                }
            }
            TreeWalkResult::Ok
        } else {
            aborted = true;
            TreeWalkResult::Abort
        }
    })?;

    if aborted {
        return Err(AppError::Missing());
    }

    Ok(())
}

pub(crate) fn difference(
    from_snapshot: PathBuf,
    to_snapshot: PathBuf,
    output: PathBuf,
) -> Result<(), AppError> {
    todo!("implement");
}

#[cfg(test)]
mod tests {
    use crate::detail::snapshot;
    use env_logger::Env;
    use git2::{ObjectType, Oid, Tree, TreeEntry, TreeWalkMode, TreeWalkResult};
    use internals::error::AppError;
    use internals::fixtures::scm::testing::TempRepo;
    use log::info;
    use std::env::consts::OS;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::{tempdir, TempDir};

    struct GitHelper {}

    impl GitHelper {
        pub fn fixture_repo() -> Result<(TempDir, PathBuf), AppError> {
            let containing_dir = tempdir()?;
            let mut repo_path = containing_dir.path().to_path_buf();
            std::env::set_current_dir(containing_dir.path());
            Command::new("git")
                .args(vec!["init", "repo"])
                .spawn()?
                .wait()
                .expect("init failed");
            repo_path.push(Path::new("repo"));
            std::env::set_current_dir(repo_path.as_path());

            let mut test_file = repo_path.to_path_buf();
            test_file.push("d_0_0");
            std::fs::create_dir(test_file.as_path());
            test_file.push("f_1.txt");
            std::fs::write(test_file.as_path(), &"This is a test file"[..]);
            test_file.pop();
            test_file.push("d_0_1");
            std::fs::create_dir(test_file.as_path());
            test_file.push("f_2.txt");
            std::fs::write(test_file.as_path(), &"This is a test file"[..]);
            test_file.pop();
            test_file.pop();
            test_file.pop();
            test_file.push("d_1_0");
            std::fs::create_dir(test_file.as_path());
            test_file.push("f_3.txt");
            std::fs::write(test_file.as_path(), &"This is a test file"[..]);

            Command::new("git")
                .args(vec!["add", "--", "."])
                .spawn()?
                .wait()
                .expect("add failed");

            Command::new("git")
                .args(vec!["commit", "-a", "-m", "Test commit"])
                .spawn()?
                .wait()
                .expect("commit failed");

            Ok((containing_dir, repo_path))
        }
    }

    fn init_logging() {
        env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    }

    struct DirNode {}

    #[test]
    fn test_snapshot() -> Result<(), AppError> {
        init_logging();

        let (containing_dir, repo_dir) = GitHelper::fixture_repo()?;
        let mut output_path = containing_dir.path().to_path_buf();
        output_path.push("output");
        snapshot(repo_dir.as_path(), output_path.as_path());
        Ok(())
    }
}
