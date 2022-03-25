use std::cell::RefCell;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fmt::Display;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::Context;
use lazy_static::lazy_static;
use regex::Regex;
use tracing::warn;

use crate::coordinate::Label;
use crate::coordinate::TargetName;
use focus_util::paths::is_relevant_to_build_graph;

use super::DependencyKey;

/// The hash of a [`DependencyKey`]'s syntactic content.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash(pub(super) git2::Oid);

impl Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self(hash) = self;
        write!(f, "{}", hash)
    }
}

impl FromStr for ContentHash {
    type Err = git2::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let oid = git2::Oid::from_str(s)?;
        Ok(ContentHash(oid))
    }
}

#[derive(Debug, Default)]
pub struct Caches {
    /// Cache of hashed dependency keys. These are only valid for the provided `repo`/`head_tree`.
    dependency_key_cache: HashMap<DependencyKey, ContentHash>,

    /// Cache of hashed tree paths, which should be either:
    ///
    ///   - Bazel-relevant build files.
    ///   - Directories.
    ///   - Non-existent.
    ///
    /// These are only valid for the provided `repo`/`head_tree`.
    tree_path_cache: HashMap<PathBuf, ContentHash>,
}

/// Context used to compute a content hash.
pub struct HashContext<'a> {
    /// The Git repository.
    pub repo: &'a git2::Repository,

    /// The tree corresponding to the current working copy.
    pub head_tree: &'a git2::Tree<'a>,

    /// Associated caches.
    pub caches: RefCell<Caches>,
}

/// Compute a content-addressable hash for the provided [`DependencyKey`] using
/// the context in `ctx`.
pub fn content_hash_dependency_key(
    ctx: &HashContext,
    key: &DependencyKey,
) -> anyhow::Result<ContentHash> {
    if let Some(hash) = ctx.caches.borrow().dependency_key_cache.get(key) {
        return Ok(hash.to_owned());
    }

    let mut buf = String::new();
    buf.push_str("DependencyKey");

    match key {
        DependencyKey::BazelPackage {
            external_repository: None,
            path,
        } => {
            buf.push_str("::BazelPackage(");
            buf.push_str(&content_hash_tree_path(ctx, path)?.to_string());

            let loaded_deps = find_load_dependencies(ctx, path)?;
            for label in loaded_deps {
                let key = DependencyKey::BazelBuildFile(label);
                buf.push_str(", ");
                buf.push_str(&content_hash_dependency_key(ctx, &key)?.to_string());
            }

            // Every package has an implicit dependency on the `WORKSPACE` file.
            let key = DependencyKey::BazelBuildFile(Label {
                external_repository: None,
                path_components: Vec::new(),
                target_name: TargetName::Name("WORKSPACE".to_string()),
            });
            buf.push_str(", ");
            buf.push_str(&content_hash_dependency_key(ctx, &key)?.to_string());
        }

        DependencyKey::Path(path) => {
            buf.push_str("::Path(");
            buf.push_str(&content_hash_tree_path(ctx, path)?.to_string());
        }

        DependencyKey::BazelPackage {
            external_repository: Some(_external_package),
            path: _,
        } => {
            todo!("establish dependency for path in external package")
        }

        DependencyKey::BazelBuildFile(label) => {
            buf.push_str("::BazelBuildFile(");
            match label {
                Label {
                    external_repository: None,
                    path_components,
                    target_name: TargetName::Name(target_name),
                } => {
                    let path: PathBuf = {
                        let mut path: PathBuf = path_components.iter().collect();
                        path.push(target_name);
                        path
                    };
                    buf.push_str(&content_hash_tree_path(ctx, &path)?.to_string());

                    // TODO: what if the `.bzl` file has been deleted?
                    let loaded_deps = match ctx.head_tree.get_path(&path) {
                        Ok(tree_entry) => {
                            extract_load_statements_from_tree_entry(ctx, &tree_entry)?
                        }
                        Err(e) if e.code() == git2::ErrorCode::NotFound => Default::default(),
                        Err(e) => return Err(e.into()),
                    };
                    for label in loaded_deps {
                        let dep_key = DependencyKey::BazelBuildFile(label);
                        buf.push_str(", ");
                        buf.push_str(&content_hash_dependency_key(ctx, &dep_key)?.to_string());
                    }
                }

                Label {
                    external_repository: None,
                    path_components,
                    target_name: TargetName::Ellipsis,
                } => {
                    warn!(
                        ?label,
                        "Got label referring to a ellipsis, but it should be a BUILD or .bzl file"
                    );
                    let path: PathBuf = path_components.iter().collect();
                    buf.push_str(&content_hash_tree_path(ctx, &path)?.to_string());
                }

                Label {
                    external_repository: Some(_),
                    path_components: _,
                    target_name: _,
                } => {
                    todo!("Implement hashing an external repository BUILD file");
                }
            }
        }
    };

    buf.push(')');
    let hash = git2::Oid::hash_object(git2::ObjectType::Blob, buf.as_bytes())?;
    let hash = ContentHash(hash);
    ctx.caches
        .borrow_mut()
        .dependency_key_cache
        .insert(key.to_owned(), hash.clone());
    Ok(hash)
}

fn content_hash_tree_path(ctx: &HashContext, path: &Path) -> anyhow::Result<ContentHash> {
    if let Some(hash) = ctx.caches.borrow().tree_path_cache.get(path) {
        return Ok(hash.to_owned());
    }

    let mut buf = String::new();
    buf.push_str("PathBuf(");

    match ctx.head_tree.get_path(path) {
        Ok(entry) => {
            buf.push_str(&entry.id().to_string());
        }
        Err(err) if err.code() == git2::ErrorCode::NotFound => {
            // TODO: test this code path
            buf.push_str(&git2::Oid::zero().to_string());
        }
        Err(err) => return Err(err.into()),
    };

    buf.push(')');
    let hash = git2::Oid::hash_object(git2::ObjectType::Blob, buf.as_bytes())?;
    let hash = ContentHash(hash);
    ctx.caches
        .borrow_mut()
        .tree_path_cache
        .insert(path.to_owned(), hash.clone());
    Ok(hash)
}

fn find_load_dependencies(
    ctx: &HashContext,
    package_path: &Path,
) -> anyhow::Result<BTreeSet<Label>> {
    let tree_entry = match ctx.head_tree.get_path(package_path) {
        Ok(tree_entry) => tree_entry,
        Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(Default::default()),
        Err(e) => return Err(e.into()),
    };
    let object = tree_entry
        .to_object(ctx.repo)
        .context("converting tree entry to object")?;
    let tree = match object.as_tree() {
        Some(tree) => tree,
        None => todo!(),
    };

    let mut result = BTreeSet::new();
    for tree_entry in tree {
        let deps = extract_load_statements_from_tree_entry(ctx, &tree_entry)?;
        result.extend(deps);
    }
    Ok(result)
}

fn extract_load_statements_from_tree_entry(
    ctx: &HashContext,
    tree_entry: &git2::TreeEntry,
) -> anyhow::Result<BTreeSet<Label>> {
    let file_name = match tree_entry.name() {
        Some(file_name) => Path::new(file_name),
        None => {
            warn!(name_bytes = ?tree_entry.name_bytes(), "Skipped tree entry with non-UTF-8 name");
            return Ok(Default::default());
        }
    };

    if !is_relevant_to_build_graph(file_name) {
        return Ok(Default::default());
    }

    let object = tree_entry
        .to_object(ctx.repo)
        .context("converting tree entry to object")?;
    let blob = match object.as_blob() {
        Some(blob) => blob,
        None => {
            warn!(?file_name, "Tree entry was not a blob");
            return Ok(Default::default());
        }
    };

    let content = match std::str::from_utf8(blob.content()) {
        Ok(content) => content,
        Err(e) => {
            warn!(?file_name, ?e, "Could not decode non-UTF-8 blob content");
            return Ok(Default::default());
        }
    };

    let deps = extract_load_statement_package_dependencies(content);
    Ok(deps)
}

fn extract_load_statement_package_dependencies(content: &str) -> BTreeSet<Label> {
    lazy_static! {
        static ref RE: Regex = Regex::new(
            r#"(?x)
# Literal "load".
load
\s*?

# Open parenthesis.
\(
\s*?

# String literal enclosed in quotes.
(?:
    "( [[:print:]--"]*? )"
  | '( [[:print:]--']*? )'
)

# Either a closing parenthesis or a comma to start the argument list.
\s*?
(?:
  ,
| \)
)
"#
        )
        .unwrap();
    }

    let mut result = BTreeSet::new();
    for cap in RE.captures_iter(content) {
        let value = cap.get(1).or_else(|| cap.get(2)).unwrap().as_str();
        let label: Label = match value.parse() {
            Ok(label) => label,
            Err(e) => {
                warn!(?e, "Failed to parse label in load statement");
                continue;
            }
        };
        result.insert(label);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_load_statements() -> anyhow::Result<()> {
        let content = r#"
load("//foo/bar:baz.bzl")
load   (
    '//foo/qux:qux.bzl'

,    qux = 'grault')
"#;
        let labels = extract_load_statement_package_dependencies(content);
        insta::assert_debug_snapshot!(labels, @r###"
        {
            Label("//foo/bar:baz.bzl"),
            Label("//foo/qux:qux.bzl"),
        }
        "###);

        Ok(())
    }
}
