use std::fmt::Display;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::Context;
use lazy_static::lazy_static;
use regex::Regex;
use sha1::digest::DynDigest;
use sha1::Digest;
use sha1::Sha1;
use tracing::warn;

use crate::coordinate::Label;
use crate::util::paths::is_relevant_to_build_graph;

use super::DependencyKey;

type Hasher<'a> = &'a mut dyn DynDigest;

/// The hash of a [`DependencyKey`]'s syntactic content.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash(git2::Oid);

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

pub struct HashContext<'a> {
    pub repo: &'a git2::Repository,
    pub head_tree: &'a git2::Tree<'a>,
}

/// Indicates that the implementing type can be content-hashed with respect to a
/// state of the repository. Callers will want to use
/// [`ContentHashable::content_hash`].
pub trait ContentHashable {
    /// Write values into the provided `hasher` according to this item's
    /// content-addressable state.
    ///
    /// In order to hash the [`ContentHashable`] values which make up the
    /// current item, the implementor can call [`ContentHashable::write`] on
    /// those values recursively.
    fn write(&self, hasher: Hasher, ctx: &HashContext) -> anyhow::Result<()>;

    /// Construct a hasher, hash this value, finalize the hash, and return the
    /// overall hash of this value.
    fn content_hash(&self, ctx: &HashContext) -> anyhow::Result<ContentHash> {
        let mut hasher = Sha1::new();
        self.write(&mut hasher, ctx)?;
        let result = hasher.finalize();
        let oid = git2::Oid::from_bytes(&result)?;
        Ok(ContentHash(oid))
    }
}

impl ContentHashable for PathBuf {
    fn write(&self, hasher: Hasher, ctx: &HashContext) -> anyhow::Result<()> {
        hasher.update(b"PathBuf(");

        match ctx.head_tree.get_path(self) {
            Ok(entry) => {
                hasher.update(entry.id().as_bytes());
            }
            Err(err) if err.code() == git2::ErrorCode::NotFound => {
                // TODO: test this code path
                hasher.update(git2::Oid::zero().as_bytes());
            }
            Err(err) => return Err(err.into()),
        };

        hasher.update(b")");
        Ok(())
    }
}

impl ContentHashable for DependencyKey {
    fn write(&self, hasher: Hasher, ctx: &HashContext) -> anyhow::Result<()> {
        hasher.update(b"DependencyKey");

        match self {
            DependencyKey::BazelPackage {
                external_repository: None,
                path,
            } => {
                hasher.update(b"::BazelPackage(");
                path.write(hasher, ctx)?;
            }

            DependencyKey::Path(path) => {
                hasher.update(b"::Path(");
                path.write(hasher, ctx)?;
            }

            DependencyKey::BazelPackage {
                external_repository: Some(_external_package),
                path: _,
            } => {
                todo!("establish dependency for path in external package")
            }

            DependencyKey::BazelBuildFile(_) => {
                // TODO: hash `BUILD` file contents
                // TODO: parse `load` dependencies out of the `BUILD` file and mix
                // into hash.
                todo!("hash bazel file dep")
            }
        };

        hasher.update(b")");
        Ok(())
    }
}

fn find_build_files(ctx: &HashContext, package_path: &Path) -> anyhow::Result<()> {
    let tree_entry = ctx.head_tree.get_path(package_path)?;
    let object = tree_entry
        .to_object(ctx.repo)
        .context("converting tree entry to object")?;
    let tree = match object.as_tree() {
        Some(tree) => tree,
        None => todo!(),
    };

    let mut result = Vec::new();
    for entry in tree {
        let file_name = match entry.name() {
            Some(file_name) => file_name,
            None => {
                warn!(?package_path, name_bytes = ?entry.name_bytes(), "Skipped tree entry with non-UTF-8 name");
                continue;
            }
        };

        if !is_relevant_to_build_graph(file_name) {
            continue;
        }
        let object = entry
            .to_object(ctx.repo)
            .context("converting tree entry to object")?;
        let blob = match object.as_blob() {
            Some(blob) => blob,
            None => {
                warn!(
                    ?package_path,
                    ?file_name,
                    "Tree entry appeared to be relevant to the build graph, but was not a blob"
                );
                continue;
            }
        };

        let content = match std::str::from_utf8(blob.content()) {
            Ok(content) => content,
            Err(e) => {
                warn!(
                    ?package_path,
                    ?file_name,
                    ?e,
                    "Could not decode non-UTF-8 blob content"
                );
                continue;
            }
        };
        result.extend(extract_load_statement_package_dependencies(content));
    }
    Ok(())
}

fn extract_load_statement_package_dependencies(content: &str) -> Vec<Label> {
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

    let mut result = Vec::new();
    for cap in RE.captures_iter(content) {
        let value = cap.get(1).or_else(|| cap.get(2)).unwrap().as_str();
        let label: Label = match value.parse() {
            Ok(label) => label,
            Err(e) => {
                warn!(?e, "Failed to parse label in load statement");
                continue;
            }
        };
        result.push(label);
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
        [
            Label("//foo/bar:baz.bzl"),
            Label("//foo/qux:qux.bzl"),
        ]
        "###);

        Ok(())
    }
}
