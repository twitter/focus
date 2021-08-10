use anyhow::Result;
use log::{info, warn};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use git2::{ObjectType, TreeEntry, TreeWalkMode, TreeWalkResult};
use internals::error::AppError;
use sha2::{Digest, Sha256};

use std::ffi::{OsString};
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
    static ref BZL_EXTENSION: OsString = OsString::from("bzl");
    static ref WORKSPACE_FILE_NAME: OsString = OsString::from("WORKSPACE");
    static ref BUILD_FILE_NAME: OsString = OsString::from("BUILD");
}

fn full_contents_required_predicate(name: &str) -> bool {
    let path = Path::new(name);
    if let Some(extension) = path.extension() {
        if extension == BZL_EXTENSION.as_os_str() {
            return true;
        }
    }

    if let Some(file_name) = path.file_name() {
        if file_name == WORKSPACE_FILE_NAME.as_os_str() || file_name == BUILD_FILE_NAME.as_os_str()
        {
            return true;
        }
    }

    false
}

pub fn snapshot(repo: &Path, _output: &Path) -> Result<(), AppError> {
    use focus_formats::treesnap::*;

    let repo = git2::Repository::open(repo)?;
    let head_reference = repo.head()?;
    let commit = head_reference.peel_to_commit()?;
    info!("Commit {:?}", commit.id());

    let mut aborted = false;
    let mut dir_stack = Vec::<String>::new();
    let mut node_stack = Vec::<bool>::new();

    let warn_of_ignored_entry = |entry: &TreeEntry| {
        let generic_object = entry.to_object(&repo).expect("Retrieving object failed");
        warn!("Ignoring {} {}", entry.kind().unwrap(), generic_object.id());
    };
    commit.tree()?.walk(TreeWalkMode::PreOrder, |path, entry| {
        if let Ok(normal_path) = normalize_tree_entry_path(path, entry) {
            let _permissions: Permissions = PermissionsExt::from_mode(entry.filemode() as u32);
            // info!("{:?} {:?} {:?}", normal_path, entry.kind(), permissions);
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
                    for _ in 0..to_pop {
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
                    let generic_object = entry.to_object(&repo).expect("Retrieving object failed");
                    let blob_object = generic_object.as_blob().expect("Conversion to blob failed");

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
                        let _blob = content::Blob {
                            content_hash: Some(content_hash),
                            content_is_inline: true,
                            content: Vec::from(content),
                        };
                    } else {
                        let _blob = content::Blob {
                            content_hash: Some(content_hash),
                            content_is_inline: false,
                            content: Vec::<u8>::new(),
                        };
                    }
                }
                Some(ObjectType::Commit) => warn_of_ignored_entry(entry),
                Some(ObjectType::Tag) => warn_of_ignored_entry(entry),
                Some(ObjectType::Any) => warn_of_ignored_entry(entry),
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
    _from_snapshot: PathBuf,
    _to_snapshot: PathBuf,
    _output: PathBuf,
) -> Result<(), AppError> {
    todo!("implement");
}

#[cfg(test)]
mod tests {
    use internals::error::AppError;

    #[test]
    fn test_snapshot() -> Result<(), AppError> {
        Ok(())
    }
}
