// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::{BTreeSet, HashSet};
use std::fmt::{Display, Write};
use std::hash::Hash;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::Context;
use lazy_static::lazy_static;
use regex::Regex;
use tracing::debug;
use tracing::error;
use tracing::trace;
use tracing::warn;

use crate::target::Label;
use crate::target::TargetName;
use focus_util::paths::is_relevant_to_build_graph;

use super::DependencyKey;

/// This value is mixed into all content hashes. Update this value when
/// content-hashing changes in a backward-incompatible way.
const VERSION: usize = 6;

/// The hash of a [`DependencyKey`]'s syntactic content.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash(pub(super) git2::Oid);

impl From<ContentHash> for git2::Oid {
    fn from(hash: ContentHash) -> Self {
        let ContentHash(oid) = hash;
        oid
    }
}

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

    /// Cache of parsed load dependencies. The OIDs here are tree OIDs which
    /// have to be traversed to find any files relevant to the build graph.
    load_dependencies_cache: HashMap<git2::Oid, BTreeSet<Label>>,

    /// Cache of dependencies loaded from the `prelude_bazel` file.
    prelude_deps_cache: Option<BTreeSet<Label>>,
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

impl std::fmt::Debug for HashContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            repo,
            head_tree,
            caches,
        } = self;
        f.debug_struct("HashContext")
            .field("repo", &repo.path())
            .field("head_tree", &head_tree.id())
            .field("caches", &caches)
            .finish()
    }
}

/// Compute a content-addressable hash for the provided [`DependencyKey`] using
/// the context in `ctx`.
pub fn content_hash(ctx: &HashContext, key: &DependencyKey) -> anyhow::Result<ContentHash> {
    content_hash_dependency_key(ctx, key, &mut HashSet::new())
}

fn content_hash_dependency_key(
    ctx: &HashContext,
    key: &DependencyKey,
    keys_being_calculated: &mut HashSet<DependencyKey>,
) -> anyhow::Result<ContentHash> {
    debug!(?key, "Hashing dependency key");

    {
        let cache = &mut ctx.caches.borrow_mut().dependency_key_cache;
        match cache.get(key) {
            Some(hash) => return Ok(hash.to_owned()),
            None => {
                if !keys_being_calculated.insert(key.clone()) {
                    anyhow::bail!(
                        "\
Circular dependency when hashing: {:?}
These are the keys currently being hashed: {:?}",
                        key,
                        keys_being_calculated.iter().collect::<BTreeSet<_>>(),
                    );
                }
            }
        }
    }

    let mut buf = String::new();
    write!(&mut buf, "DependencyKeyV{VERSION}")?;

    match key {
        DependencyKey::BazelPackage(
            label @ Label {
                external_repository,
                path_components,
                target_name: _,
            },
        ) => {
            buf.push_str("::BazelPackage(");
            buf.push_str(&label.to_string());

            buf.push(',');
            let path: PathBuf = path_components.iter().collect();
            buf.push_str(&content_hash_tree_path(ctx, &path)?.to_string());

            if external_repository.is_none() {
                let mut loaded_deps = match get_tree_for_path(ctx, &path)? {
                    Some(tree) => find_load_dependencies(ctx, &tree)?,
                    None => Default::default(),
                };

                let prelude_deps = get_prelude_deps(ctx)?;
                loaded_deps.extend(prelude_deps);

                for label in loaded_deps {
                    let key = DependencyKey::BazelBuildFile(label);
                    buf.push_str(", ");
                    buf.push_str(
                        &content_hash_dependency_key(ctx, &key, keys_being_calculated)?.to_string(),
                    );
                }
            }

            // Every package has an implicit dependency on the `WORKSPACE` file.
            let key = DependencyKey::BazelBuildFile(Label {
                external_repository: None,
                path_components: Vec::new(),
                target_name: TargetName::Name("WORKSPACE".to_string()),
            });
            buf.push_str(", ");
            buf.push_str(
                &content_hash_dependency_key(ctx, &key, keys_being_calculated)?.to_string(),
            );
        }

        DependencyKey::BazelBuildFile(
            label @ Label {
                external_repository,
                path_components,
                target_name,
            },
        ) => {
            buf.push_str("::BazelBuildFile(");
            buf.push_str(&label.to_string());

            if external_repository.is_none() {
                match target_name {
                    TargetName::Name(target_name) => {
                        let path: PathBuf = {
                            let mut path: PathBuf = path_components.iter().collect();
                            path.push(target_name);
                            path
                        };
                        buf.push_str(", ");
                        buf.push_str(&content_hash_tree_path(ctx, &path)?.to_string());

                        let loaded_deps = match ctx.head_tree.get_path(&path) {
                            Ok(tree_entry) => {
                                if is_tree_entry_relevant_to_build_graph(&tree_entry) {
                                    extract_load_statements_from_tree_entry(ctx, &tree_entry)?
                                } else {
                                    Default::default()
                                }
                            }
                            Err(e) if e.code() == git2::ErrorCode::NotFound => Default::default(),
                            Err(e) => return Err(e.into()),
                        };
                        for label in loaded_deps {
                            let dep_key = DependencyKey::BazelBuildFile(label);

                            // HACK: one of our `.bzl` files includes a
                            // reference to its own label in a doc-comment,
                            // which causes it to be picked up by our naive
                            // build file parser. Until the build file parser is
                            // hardened, this suffices to parse the rest of the
                            // `.bzl` files in our projects.
                            if key == &dep_key {
                                continue;
                            }

                            buf.push_str(", ");
                            buf.push_str(
                                &content_hash_dependency_key(ctx, &dep_key, keys_being_calculated)?
                                    .to_string(),
                            );
                        }
                    }

                    TargetName::Ellipsis => {
                        anyhow::bail!(
                            "Got label referring to a ellipsis, but it should be a BUILD or .bzl file: {:?}",
                            label
                        );
                    }
                }
            }

            // Every `.bzl` file (or similar) has an implicit dependency on the
            // `WORKSPACE` file. However, the `WORKSPACE` file itself may `load`
            // `.bzl` files in the repository. To avoid a circular dependency,
            // use only the textual hash of the WORKSPACE as the dependency key
            // here.
            let workspace_key = DependencyKey::Path("WORKSPACE".into());
            buf.push_str(", ");
            buf.push_str(
                &content_hash_dependency_key(ctx, &workspace_key, keys_being_calculated)?
                    .to_string(),
            );
        }

        DependencyKey::Path(path) => {
            buf.push_str("::Path(");
            buf.push_str(&content_hash_tree_path(ctx, path)?.to_string());
        }

        DependencyKey::DummyForTesting(inner_dep_key) => {
            buf.push_str("DummyForTesting(");
            buf.push_str(
                &content_hash_dependency_key(ctx, inner_dep_key.borrow(), keys_being_calculated)?
                    .to_string(),
            );
        }
    };

    buf.push(')');
    let hash = git2::Oid::hash_object(git2::ObjectType::Blob, buf.as_bytes())?;
    let hash = ContentHash(hash);
    if let Some(old_value) = ctx
        .caches
        .borrow_mut()
        .dependency_key_cache
        .insert(key.to_owned(), hash.clone())
    {
        if old_value != hash {
            error!(?key, ?old_value, new_value = ?hash, "Non-deterministic content hashing for dependency key");
        }
    }
    Ok(hash)
}

/// Get the dependencies induced by the special
/// `tools/build_rules/prelude_bazel` file (if present). See
/// https://github.com/bazelbuild/bazel/issues/1674 for discussion on what this
/// file is.
pub fn get_prelude_deps(ctx: &HashContext) -> anyhow::Result<BTreeSet<Label>> {
    if let Some(prelude_deps) = &ctx.caches.borrow().prelude_deps_cache {
        return Ok(prelude_deps.clone());
    }

    let prelude_dir = ["tools", "build_rules"];
    let prelude_file_name = "prelude_bazel";
    let prelude_path: PathBuf = prelude_dir.into_iter().chain([prelude_file_name]).collect();

    let result = match ctx.head_tree.get_path(&prelude_path) {
        Ok(tree_entry) => {
            let mut result = BTreeSet::new();
            result.insert(Label {
                external_repository: None,
                path_components: prelude_dir.into_iter().map(|s| s.to_string()).collect(),
                target_name: TargetName::Name(prelude_file_name.to_string()),
            });
            result.extend(extract_load_statements_from_tree_entry(ctx, &tree_entry)?);
            result
        }
        Err(err) if err.code() == git2::ErrorCode::NotFound => Default::default(),
        Err(err) => return Err(err.into()),
    };

    ctx.caches.borrow_mut().prelude_deps_cache = Some(result.clone());
    Ok(result)
}

fn content_hash_tree_path(ctx: &HashContext, path: &Path) -> anyhow::Result<ContentHash> {
    if let Some(hash) = ctx.caches.borrow().tree_path_cache.get(path) {
        return Ok(hash.clone());
    }

    let mut buf = String::new();
    write!(
        &mut buf,
        "PathBufV{VERSION}({tree_id})",
        tree_id = get_tree_path_id(ctx.head_tree, path)?
    )?;

    let hash = git2::Oid::hash_object(git2::ObjectType::Blob, buf.as_bytes())?;
    let hash = ContentHash(hash);
    if let Some(old_value) = ctx
        .caches
        .borrow_mut()
        .tree_path_cache
        .insert(path.to_owned(), hash.clone())
    {
        if old_value != hash {
            error!(key = ?path, ?old_value, new_value = ?hash, "Non-deterministic content hashing for path");
        }
    }
    Ok(hash)
}

fn get_tree_path_id(tree: &git2::Tree, path: &Path) -> Result<git2::Oid, git2::Error> {
    if path == Path::new("") {
        // `get_path` will produce an error if we pass an empty path, so
        // manually handle that here.
        Ok(tree.id())
    } else {
        match tree.get_path(path) {
            Ok(entry) => Ok(entry.id()),
            Err(err) if err.code() == git2::ErrorCode::NotFound => {
                // TODO: test this code path
                Ok(git2::Oid::zero())
            }
            Err(err) => Err(err),
        }
    }
}

fn get_tree_for_path<'repo>(
    ctx: &HashContext<'repo>,
    package_path: &Path,
) -> anyhow::Result<Option<git2::Tree<'repo>>> {
    if package_path == Path::new("") {
        Ok(Some(ctx.head_tree.to_owned()))
    } else {
        let tree_entry = match ctx.head_tree.get_path(package_path) {
            Ok(tree_entry) => tree_entry,
            Err(e) if e.code() == git2::ErrorCode::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let object = tree_entry
            .to_object(ctx.repo)
            .context("converting tree entry to object")?;
        let tree = object.as_tree().map(|tree| tree.to_owned());
        Ok(tree)
    }
}

fn find_load_dependencies(ctx: &HashContext, tree: &git2::Tree) -> anyhow::Result<BTreeSet<Label>> {
    trace!(?tree, "Finding load dependencies");
    if let Some(result) = ctx.caches.borrow().load_dependencies_cache.get(&tree.id()) {
        return Ok(result.clone());
    }

    let mut result = BTreeSet::new();
    for tree_entry in tree {
        if is_tree_entry_relevant_to_build_graph(&tree_entry) {
            let deps = extract_load_statements_from_tree_entry(ctx, &tree_entry)?;
            result.extend(deps);
        }
    }
    if let Some(old_value) = ctx
        .caches
        .borrow_mut()
        .load_dependencies_cache
        .insert(tree.id(), result.clone())
    {
        if old_value != result {
            error!(key = ?tree.id(), ?old_value, new_value = ?result, "Non-deterministic content hashing for load dependencies");
        }
    }
    Ok(result)
}

fn is_tree_entry_relevant_to_build_graph(tree_entry: &git2::TreeEntry) -> bool {
    match tree_entry.name() {
        Some(file_name) => is_relevant_to_build_graph(Path::new(file_name)),
        None => {
            warn!(name_bytes = ?tree_entry.name_bytes(), "Skipped tree entry with non-UTF-8 name");
            false
        }
    }
}

fn extract_load_statements_from_tree_entry(
    ctx: &HashContext,
    tree_entry: &git2::TreeEntry,
) -> anyhow::Result<BTreeSet<Label>> {
    let object = tree_entry
        .to_object(ctx.repo)
        .context("converting tree entry to object")?;
    let blob = match object.as_blob() {
        Some(blob) => blob,
        None => {
            warn!(file_name = ?tree_entry.name(), "Tree entry was not a blob");
            return Ok(Default::default());
        }
    };

    let content = match std::str::from_utf8(blob.content()) {
        Ok(content) => content,
        Err(e) => {
            warn!(file_name = ?tree_entry.name(), ?e, "Could not decode non-UTF-8 blob content");
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
