// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;

use crate::index::content_hash::get_prelude_deps;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::target::{Label, Target, TargetName};
use crate::target_resolver::ResolutionResult;

use super::content_hash::HashContext;
use super::{ContentHash, ObjectDatabase};

/// A key into the "Focus Build Graph" which lets us identify a corresponding
/// [`DependencyValue`] node.  A [`DependencyKey`] in combination with a
/// snapshot of the repository is *syntactically* content-addressable in this
/// sense: the hash of the [`DependencyKey`] can be calculated only by looking
/// at file contents, without having to evaluate any Bazel queries.
///
/// The objective is to store the key-value pair in persistent storage, and use
/// the key for lookups via content hashing. The corresponding value, if
/// present, would be a cached version of the dependencies as evaluated by
/// Bazel.
///
/// A [`DependencyKey`] tells us two things:
///
/// - Where in the repository we can find the file contents to be hashed.
///   - If the content of this dependency has changed, then we may need to issue
///   a Bazel query to figure out the new dependencies to materialize.
/// - What other dependencies/content could potentially invalidate the meaning of this
/// content.
///   - It's possible for the meaning of this content to change even if the
///   literal file contents have not changed. See
///   `DependencyKey::BazelBuildFile`.
///
/// Note that a [`DependencyKey`] by itself (without accompanying repository
/// state) is not sufficient to compute a content hash. However, it suffices as
/// a logical name for an entity in the repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DependencyKey {
    /// Represents a dependency on a Bazel package in the Bazel Build Graph.
    ///
    /// For example, if `//foo` depends on `//bar`, then we need to materialize
    /// both `//foo` and `//bar`'s files.
    ///
    /// TODO: explain what happens in the cases of ellipses
    BazelPackage(Label),

    /// Represents a dependency on a `BUILD` or `.bzl` file.
    ///
    /// For example, if `/foo/BUILD` has a load statement like
    ///
    /// ```python
    /// load("bar.bzl")
    /// ```
    ///
    /// then the meaning of the `BUILD` file may change whenever `bar.bzl`
    /// changes, even if the contents of the `BUILD` file don't change. Thus, we
    /// need to read the contents of *that* `.bzl` file and mix them into the
    /// hash for this dependency.
    ///
    /// NOTE: currently, the Bazel resolver does not produce any
    /// `BazelBuildFile` dependencies. Instead, it produces the `BazelPackage`
    /// which contains those `.bzl` files. So the only uses of this are where we
    /// manually construct it as part of content hashing.
    BazelBuildFile(Label),

    /// Represents a path (probably a directory) which should be checked out as
    /// part of the sparse checkout.
    ///
    /// Example: a directory containing configuration files or other assets, but
    /// which isn't a Bazel package.
    Path(PathBuf),

    /// This value was generated during testing, and should not appear in a
    /// production object database.
    DummyForTesting(Box<DependencyKey>),
}

impl From<Target> for DependencyKey {
    fn from(target: Target) -> Self {
        match target {
            Target::Bazel(label) => Self::BazelPackage(label),
            Target::Directory(path) => Self::Path(PathBuf::from(path)),
            Target::Pants(label) => unimplemented!(
                "DependencyKey from Pants label not supported (label: {})",
                label
            ),
        }
    }
}

/// The semantic content associated with a [`DependencyKey`], produced by
/// expensive operations such as querying Bazel.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DependencyValue {
    /// The provided [`DependencyKey`] represented a Bazel package.
    PackageInfo {
        /// The immediate dependencies for this package.  Transitive
        /// dependencies are not included; the caller will need to do its own
        /// traversal to accumulate them.
        deps: BTreeSet<DependencyKey>,
    },

    /// The provided [`DependencyKey`] indicates that a given path (not
    /// necessarily corresponding to a Bazel package) should be checked out on
    /// disk.
    Path {
        /// The path to check out. (This is most likely a directory.)
        path: PathBuf,
    },

    /// This value was generated during testing, and should not appear in a
    /// production object database.
    DummyForTesting(DependencyKey),
}

/// Add content-addressable key-value pairs corresponding to the calculated
/// dependencies to the [`ObjectDatabase`].
pub fn update_object_database_from_resolution(
    ctx: &HashContext,
    odb: &dyn ObjectDatabase,
    resolution_result: &ResolutionResult,
) -> anyhow::Result<()> {
    debug!(
        ?resolution_result,
        "Updating object database from ResolutionResult"
    );
    let ResolutionResult {
        paths: _,
        package_deps,
    } = resolution_result;

    for (dep_key, dep_value) in package_deps {
        match dep_key {
            DependencyKey::BazelPackage { .. } | DependencyKey::BazelBuildFile(_) => {
                // Do nothing.
            }
            DependencyKey::Path(_) | DependencyKey::DummyForTesting(_) => {
                debug!(
                    ?dep_key,
                    "Non-Bazel dependency key returned in `ResolutionResult`"
                )
            }
        }

        odb.put(ctx, dep_key, dep_value.clone())?;
    }
    Ok(())
}

/// The result of determining which paths should be materialized according to
/// the user's focused packages.
#[derive(Clone, Debug)]
pub enum PathsToMaterializeResult {
    /// The set of paths to materialize was successfully determined.
    Ok {
        /// *All* dependency keys encountered in the course of materializing
        /// paths. This includes the starting set of keys passed in and its
        /// transitively-reachable closure.
        seen_keys: BTreeSet<DependencyKey>,

        /// The set of files/directories which should be materialized.
        paths: BTreeSet<PathBuf>,
    },

    /// Some entries were missing from the [`ObjectDatabase`], so the set of
    /// paths to materialize could not be determined using only index lookups.
    MissingKeys {
        /// The keys which were queried but absent.
        missing_keys: BTreeSet<(DependencyKey, ContentHash)>,

        /// All the keys which were queried.
        seen_keys: BTreeSet<DependencyKey>,
    },
}

fn label_into_path(label: Label) -> Option<PathBuf> {
    match label {
        Label {
            external_repository: Some(_),
            path_components: _,
            target_name: _,
        } => {
            // This is an external repository. We don't need to materialize any
            // extra files on disk for it; that should be handled by Bazel
            // itself.
            None
        }

        Label {
            external_repository: None,
            path_components,
            target_name: TargetName::Ellipsis,
        } => Some(path_components.into_iter().collect()),

        Label {
            external_repository: None,
            path_components,
            target_name: TargetName::Name(name),
        } => {
            let mut path: PathBuf = path_components.into_iter().collect();
            path.push(name);
            Some(path)
        }
    }
}

/// Given a set of packages which are currently focused, determine which paths
/// need to be checked out in the sparse repository to support building those
/// packages. This uses the [`ObjectDatabase`] and avoids querying Bazel or the
/// working copy.
pub fn get_files_to_materialize(
    ctx: &HashContext,
    odb: &dyn ObjectDatabase,
    dep_keys: HashSet<DependencyKey>,
) -> anyhow::Result<PathsToMaterializeResult> {
    let mut dep_keys = dep_keys;
    debug!(?dep_keys, "Initial set of dependency keys");

    // The result of `bazel query` appears to not include dependencies that are
    // caused by `prelude_bazel`, so we have to manually add them as part of the
    // file materialization process.
    let prelude_deps = get_prelude_deps(ctx)?;
    debug!(?prelude_deps, "Prelude deps");
    dep_keys.extend(prelude_deps.into_iter().map(DependencyKey::BazelBuildFile));

    // Recursively resolve each dependency's content hashes.
    let mut paths_to_materialize = HashSet::new();
    let mut seen_keys = HashSet::new();
    let mut missing_keys = HashSet::new();
    while !dep_keys.is_empty() {
        let mut next_deps = HashSet::new();
        for dep_key in dep_keys {
            seen_keys.insert(dep_key.clone());

            match &dep_key {
                DependencyKey::BazelPackage(Label {
                    external_repository: None,
                    path_components,
                    // Ignore the target name. We want to materialize the entire directory.
                    target_name: _,
                }) => {
                    let path: PathBuf = path_components.iter().collect();
                    paths_to_materialize.insert(path);
                }

                DependencyKey::Path(path) => {
                    paths_to_materialize.insert(path.clone());
                    continue;
                }

                DependencyKey::BazelPackage(Label {
                    external_repository: Some(_),
                    path_components: _,
                    target_name: _,
                }) => {
                    // Do nothing, we expect Bazel itself to have loaded
                    // external packages.
                    // TODO: run `bazel sync` to ensure that?
                    continue;
                }

                DependencyKey::BazelBuildFile(label) => {
                    let containing_package = Label {
                        target_name: TargetName::Ellipsis,
                        ..label.clone()
                    };
                    let path = label_into_path(containing_package);
                    if let Some(path) = path {
                        paths_to_materialize.insert(path);
                    }
                    continue;
                }

                DependencyKey::DummyForTesting(inner_dep_key) => {
                    warn!(
                        ?inner_dep_key,
                        "Encountered dummy testing key; this should not appear in real-world data"
                    );
                    continue;
                }
            };

            let (dep_hash, dep_value) = odb.get(ctx, &dep_key)?;
            debug!(
                ?dep_hash,
                ?dep_key,
                ?dep_value,
                "Looked up dep value from key"
            );

            match dep_value {
                Some(DependencyValue::PackageInfo { deps }) => {
                    next_deps.extend(deps.into_iter());
                }

                Some(DependencyValue::Path { path }) => {
                    paths_to_materialize.insert(path);
                }

                Some(DependencyValue::DummyForTesting(inner_dep_key)) => {
                    warn!(?inner_dep_key, "Encountered dummy testing value; this should not appear in real-world data");
                }

                None => {
                    missing_keys.insert((dep_key, dep_hash));
                }
            }
        }

        debug!(
            ?next_deps,
            "Next set of dependency hashes, before deduplication"
        );
        for hash in seen_keys.iter() {
            next_deps.remove(hash);
        }
        debug!(
            ?next_deps,
            "Next set of dependency hashes, after deduplication"
        );
        dep_keys = next_deps;
    }

    if missing_keys.is_empty() {
        Ok(PathsToMaterializeResult::Ok {
            seen_keys: seen_keys.into_iter().collect(),
            paths: paths_to_materialize.into_iter().collect(),
        })
    } else {
        Ok(PathsToMaterializeResult::MissingKeys {
            missing_keys: missing_keys.into_iter().collect(),
            seen_keys: seen_keys.into_iter().collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use maplit::hashset;

    use crate::index::object_database::{testing::HashMapOdb, RocksDBCache};
    use crate::index::RocksDBMemoizationCacheExt;
    use crate::target::Target;
    use crate::target_resolver::{BazelResolver, CacheOptions, ResolutionRequest, Resolver};
    use focus_testing::init_logging;
    use focus_testing::ScratchGitRepo;
    use focus_util::app::App;

    use super::*;

    fn write_files(fix: &ScratchGitRepo, files: &str) -> anyhow::Result<()> {
        let files = files
            .trim()
            .split("file: ")
            .filter_map(|file_contents| file_contents.split_once('\n'));
        for (file_name, file_contents) in files {
            fix.write_file(file_name, file_contents.trim())?;
            fix.add_file(file_name)?;
        }
        Ok(())
    }

    fn parse_label(label: &str) -> anyhow::Result<DependencyKey> {
        let target = Target::try_from(format!("bazel:{}", label).as_str())?;
        let dep_key = DependencyKey::from(target);
        Ok(dep_key)
    }

    #[test]
    fn test_get_files_to_materialize() -> anyhow::Result<()> {
        init_logging();

        let temp = tempfile::tempdir()?;
        let fix = ScratchGitRepo::new_static_fixture(temp.path())?;

        write_files(
            &fix,
            r#"
file: WORKSPACE

file: package1/foo.sh
#!/bin/sh
echo "Hello, world!"

file: package1/BUILD
sh_binary(
    name = "foo",
    srcs = ["foo.sh"],
    deps = ["//package2:bar"],
    tags = ["bazel-compatible"],
)

file: package2/bar.sh
#!/bin/sh
echo "Loaded dependency contents"

file: package2/BUILD
sh_binary(
    name = "bar",
    srcs = ["bar.sh"],
    tags = ["bazel-compatible"],
)
"#,
        )?;
        let head_oid = fix.commit_all("Wrote files")?;

        let repo = fix.repo()?;
        let odb = RocksDBCache::new(&repo);
        let files_to_materialize = {
            let head_commit = repo.find_commit(head_oid)?;
            let head_tree = head_commit.tree()?;
            let ctx = HashContext::new(&repo, &head_tree)?;
            get_files_to_materialize(&ctx, &odb, hashset! { parse_label("//package1:foo")? })?
        };
        // Confirm that the object for package1 is not yet in the database.
        insta::assert_debug_snapshot!(files_to_materialize, @r###"
        MissingKeys {
            missing_keys: {
                (
                    BazelPackage(
                        Label("//package1:foo"),
                    ),
                    ContentHash(
                        88ac08d5fc3ad9e7c278929a6c649f113e15686c,
                    ),
                ),
            },
            seen_keys: {
                BazelPackage(
                    Label("//package1:foo"),
                ),
            },
        }
        "###);

        let app = Arc::new(App::new_for_testing()?);
        let cache_dir = tempfile::tempdir()?;
        let resolver = BazelResolver::new(cache_dir.path());
        let target_set =
            hashset! {"bazel://package1:foo".try_into()?, "bazel://package2:bar".try_into()?};
        let request = ResolutionRequest {
            repo: fix.path().to_path_buf(),
            targets: target_set,
        };
        let cache_options = CacheOptions::default();
        let resolve_result = resolver.resolve(&request, &cache_options, app)?;
        insta::assert_debug_snapshot!(resolve_result, @r###"
        ResolutionResult {
            paths: {
                "package1",
                "package2",
            },
            package_deps: {
                BazelPackage(
                    Label("//package1:foo"),
                ): PackageInfo {
                    deps: {
                        BazelPackage(
                            Label("//package1:foo.sh"),
                        ),
                        BazelPackage(
                            Label("//package2:bar"),
                        ),
                    },
                },
                BazelPackage(
                    Label("//package1:foo.sh"),
                ): PackageInfo {
                    deps: {},
                },
                BazelPackage(
                    Label("//package2:bar"),
                ): PackageInfo {
                    deps: {
                        BazelPackage(
                            Label("//package2:bar.sh"),
                        ),
                    },
                },
                BazelPackage(
                    Label("//package2:bar.sh"),
                ): PackageInfo {
                    deps: {},
                },
            },
        }
        "###);

        let repo = fix.repo()?;
        let head_commit = repo.find_commit(head_oid)?;
        let head_tree = head_commit.tree()?;
        let ctx = HashContext::new(&repo, &head_tree)?;
        update_object_database_from_resolution(&ctx, &odb, &resolve_result)?;
        let files_to_materialize =
            get_files_to_materialize(&ctx, &odb, hashset! { parse_label("//package1:foo")? })?;
        insta::assert_debug_snapshot!(files_to_materialize, @r###"
        Ok {
            seen_keys: {
                BazelPackage(
                    Label("//package1:foo"),
                ),
                BazelPackage(
                    Label("//package1:foo.sh"),
                ),
                BazelPackage(
                    Label("//package2:bar"),
                ),
                BazelPackage(
                    Label("//package2:bar.sh"),
                ),
            },
            paths: {
                "package1",
                "package2",
            },
        }
        "###);

        Ok(())
    }

    #[test]
    fn test_bzl_file_dependency() -> anyhow::Result<()> {
        init_logging();

        let temp = tempfile::tempdir()?;
        let fix = ScratchGitRepo::new_static_fixture(temp.path())?;

        write_files(
            &fix,
            r#"
file: WORKSPACE

file: macro/BUILD

file: macro/macro.bzl
load("//macro:macro_inner.bzl", "my_macro_inner")
def my_macro(name):
    my_macro_inner(name)

file: macro/macro_inner.bzl
def my_macro_inner(name):
    native.genrule(
        name = name,
        srcs = ["//package2:contents"],
        tags = ["bazel-compatible"],
        outs = ["out.txt"],
        cmd = "cp $(SRCS) $@",
    )

file: package1/BUILD
load("//macro:macro.bzl", "my_macro")
my_macro("foo")

file: package2/BUILD
exports_files(["contents"])

file: package2/contents
Old contents

file: package3/BUILD
exports_files(["contents"])

file: package3/contents
New contents
"#,
        )?;
        let head_oid = fix.commit_all("Wrote files")?;
        let repo = fix.repo()?;

        let app = Arc::new(App::new_for_testing()?);
        let cache_dir = tempfile::tempdir()?;
        let resolver = BazelResolver::new(cache_dir.path());
        let target_set = hashset! {"bazel://package1:foo".try_into()? };
        let request = ResolutionRequest {
            repo: fix.path().to_path_buf(),
            targets: target_set,
        };
        let cache_options = CacheOptions::default();
        let resolve_result = resolver.resolve(&request, &cache_options, app.clone())?;

        let odb = RocksDBCache::new(&repo);
        let files_to_materialize = {
            let head_commit = repo.find_commit(head_oid)?;
            let head_tree = head_commit.tree()?;
            let hash_context = HashContext::new(&repo, &head_tree)?;
            update_object_database_from_resolution(&hash_context, &odb, &resolve_result)?;

            get_files_to_materialize(
                &hash_context,
                &odb,
                hashset! { parse_label("//package1:foo")? },
            )?
        };
        insta::assert_debug_snapshot!(files_to_materialize, @r###"
        Ok {
            seen_keys: {
                BazelPackage(
                    Label("//package1:foo"),
                ),
                BazelPackage(
                    Label("//package2:contents"),
                ),
            },
            paths: {
                "package1",
                "package2",
            },
        }
        "###);

        // Make a change that should invalidate the macro loaded by
        // `package1/BUILD`. If it was not correctly invalidated, then the call
        // to [`get_files_to_materialize`] would return the same result as
        // before.
        let head_oid = fix.write_and_commit_file(
            "macro/macro_inner.bzl",
            r#"\
def my_macro_inner(name):
    native.genrule(
        name = name,
        srcs = ["//package3:contents"],
        tags = ["bazel-compatible"],
        outs = ["out.txt"],
        cmd = "cp $(SRCS) $@",
    )
"#,
            "update macro.bzl",
        )?;
        let files_to_materialize = {
            let head_commit = repo.find_commit(head_oid)?;
            let head_tree = head_commit.tree()?;
            let hash_context = HashContext::new(&repo, &head_tree)?;
            get_files_to_materialize(
                &hash_context,
                &odb,
                hashset! { parse_label("//package1:foo")? },
            )?
        };
        insta::assert_debug_snapshot!(files_to_materialize, @r###"
        MissingKeys {
            missing_keys: {
                (
                    BazelPackage(
                        Label("//package1:foo"),
                    ),
                    ContentHash(
                        8d824d3f5ea161a4532e45a8a2686a8b196ac461,
                    ),
                ),
            },
            seen_keys: {
                BazelPackage(
                    Label("//package1:foo"),
                ),
            },
        }
        "###);

        let resolve_result = resolver.resolve(&request, &cache_options, app)?;
        let files_to_materialize = {
            let head_commit = repo.find_commit(head_oid)?;
            let head_tree = head_commit.tree()?;
            let hash_context = HashContext::new(&repo, &head_tree)?;
            update_object_database_from_resolution(&hash_context, &odb, &resolve_result)?;
            get_files_to_materialize(
                &hash_context,
                &odb,
                hashset! { parse_label("//package1:foo")? },
            )?
        };
        insta::assert_debug_snapshot!(files_to_materialize, @r###"
        Ok {
            seen_keys: {
                BazelPackage(
                    Label("//package1:foo"),
                ),
                BazelPackage(
                    Label("//package3:contents"),
                ),
            },
            paths: {
                "package1",
                "package3",
            },
        }
        "###);

        Ok(())
    }

    #[test]
    fn test_workspace_dependency() -> anyhow::Result<()> {
        init_logging();

        let temp = tempfile::tempdir()?;
        let fix = ScratchGitRepo::new_static_fixture(temp.path())?;

        write_files(
            &fix,
            r#"
file: WORKSPACE
load("//macro:macro.bzl", "some_macro")
some_macro()

file: package1/foo.sh

file: package1/BUILD
sh_binary(
    name = "foo",
    srcs = ["foo.sh"],
    tags = ["bazel-compatible"],
)

file: macro/BUILD

file: macro/macro.bzl
def some_macro():
    pass
"#,
        )?;
        let head_oid = fix.commit_all("Wrote files")?;
        let repo = fix.repo()?;

        let app = Arc::new(App::new_for_testing()?);
        let cache_dir = tempfile::tempdir()?;
        let resolver = BazelResolver::new(cache_dir.path());
        let target_set = hashset! {"bazel://package1:foo".try_into()?};
        let request = ResolutionRequest {
            repo: fix.path().to_path_buf(),
            targets: target_set,
        };
        let cache_options = CacheOptions::default();
        let resolve_result = resolver.resolve(&request, &cache_options, app)?;

        let odb = HashMapOdb::new();
        let files_to_materialize = {
            let head_commit = repo.find_commit(head_oid)?;
            let head_tree = head_commit.tree()?;
            let ctx = HashContext::new(&repo, &head_tree)?;
            update_object_database_from_resolution(&ctx, &odb, &resolve_result)?;
            get_files_to_materialize(&ctx, &odb, hashset! { parse_label("//package1:foo")? })?
        };
        insta::assert_debug_snapshot!(files_to_materialize, @r###"
        Ok {
            seen_keys: {
                BazelPackage(
                    Label("//package1:foo"),
                ),
                BazelPackage(
                    Label("//package1:foo.sh"),
                ),
            },
            paths: {
                "package1",
            },
        }
        "###);

        let files_to_materialize = {
            let head_oid = fix.write_and_commit_file(
                "macro/macro.bzl",
                r#"
def some_macro():
    # touch this file
    pass
"#,
                "update macro.bzl",
            )?;
            let head_commit = repo.find_commit(head_oid)?;
            let head_tree = head_commit.tree()?;
            let hash_context = HashContext::new(&repo, &head_tree)?;
            get_files_to_materialize(
                &hash_context,
                &odb,
                hashset! { parse_label("//package1:foo")? },
            )?
        };
        insta::assert_debug_snapshot!(files_to_materialize, @r###"
        MissingKeys {
            missing_keys: {
                (
                    BazelPackage(
                        Label("//package1:foo"),
                    ),
                    ContentHash(
                        d340b3bb64561cebebe211ff4870602a6e385393,
                    ),
                ),
            },
            seen_keys: {
                BazelPackage(
                    Label("//package1:foo"),
                ),
            },
        }
        "###);

        Ok(())
    }

    #[test]
    fn test_prelude_bazel_dependency() -> anyhow::Result<()> {
        init_logging();

        let temp = tempfile::tempdir()?;
        let fix = ScratchGitRepo::new_static_fixture(temp.path())?;

        write_files(
            &fix,
            r#"
file: WORKSPACE

file: tools/build_rules/BUILD

file: tools/build_rules/prelude_bazel
load("//macro:macro.bzl", "macro")

file: macro/BUILD

file: macro/macro.bzl
def macro(name):
    native.genrule(
        name = name,
        outs = ["out.txt"],
        cmd = "echo hi >$@",
    )

file: package1/BUILD
macro("foo")
"#,
        )?;
        let head_oid = fix.commit_all("Wrote files")?;
        let repo = fix.repo()?;

        let app = Arc::new(App::new_for_testing()?);
        let cache_dir = tempfile::tempdir()?;
        let resolver = BazelResolver::new(cache_dir.path());
        let cache_options = CacheOptions::default();

        let odb = HashMapOdb::new();
        let files_to_materialize = {
            let head_commit = repo.find_commit(head_oid)?;
            let head_tree = head_commit.tree()?;
            let ctx = HashContext::new(&repo, &head_tree)?;

            let target_set = hashset! { "bazel://package1:foo".try_into()? };
            let request = ResolutionRequest {
                repo: fix.path().to_path_buf(),
                targets: target_set,
            };
            let resolve_result = resolver.resolve(&request, &cache_options, app.clone())?;
            update_object_database_from_resolution(&ctx, &odb, &resolve_result)?;
            get_files_to_materialize(&ctx, &odb, hashset! { parse_label("//package1:foo")? })?
        };
        insta::assert_debug_snapshot!(files_to_materialize, @r###"
        Ok {
            seen_keys: {
                BazelPackage(
                    Label("//package1:foo"),
                ),
                BazelBuildFile(
                    Label("//macro:macro.bzl"),
                ),
                BazelBuildFile(
                    Label("//tools/build_rules:prelude_bazel"),
                ),
            },
            paths: {
                "macro",
                "package1",
                "tools/build_rules",
            },
        }
        "###);

        write_files(
            &fix,
            r#"
file: macro/macro.bzl
load("//macro2:macro2.bzl", "macro2")
def macro(name):
    macro2(name)

file: macro2/BUILD

file: macro2/macro2.bzl
def macro2(name):
    native.genrule(
        name = name + "2",
        outs = ["out.txt"],
        cmd = "echo hi >$@",
    )

"#,
        )?;
        let head_oid = fix.commit_all("Wrote files")?;
        let (old_files_to_materialize, new_files_to_materialize) = {
            let head_commit = repo.find_commit(head_oid)?;
            let head_tree = head_commit.tree()?;
            let ctx = HashContext::new(&repo, &head_tree)?;
            let old_files_to_materialize =
                get_files_to_materialize(&ctx, &odb, hashset! { parse_label("//package1:foo")? })?;

            let resolve_result = {
                let target_set = hashset! { "bazel://package1:foo2".try_into()? };
                let request = ResolutionRequest {
                    repo: fix.path().to_path_buf(),
                    targets: target_set,
                };
                resolver.resolve(&request, &cache_options, app)?
            };
            update_object_database_from_resolution(&ctx, &odb, &resolve_result)?;

            let new_files_to_materialize =
                get_files_to_materialize(&ctx, &odb, hashset! { parse_label("//package1:foo2")? })?;
            (old_files_to_materialize, new_files_to_materialize)
        };
        insta::assert_debug_snapshot!(old_files_to_materialize, @r###"
        MissingKeys {
            missing_keys: {
                (
                    BazelPackage(
                        Label("//package1:foo"),
                    ),
                    ContentHash(
                        92e8d6005643b9d223d1196651c00da334edcb2a,
                    ),
                ),
            },
            seen_keys: {
                BazelPackage(
                    Label("//package1:foo"),
                ),
                BazelBuildFile(
                    Label("//macro:macro.bzl"),
                ),
                BazelBuildFile(
                    Label("//tools/build_rules:prelude_bazel"),
                ),
            },
        }
        "###);
        insta::assert_debug_snapshot!(new_files_to_materialize, @r###"
        Ok {
            seen_keys: {
                BazelPackage(
                    Label("//package1:foo2"),
                ),
                BazelBuildFile(
                    Label("//macro:macro.bzl"),
                ),
                BazelBuildFile(
                    Label("//tools/build_rules:prelude_bazel"),
                ),
            },
            paths: {
                "macro",
                "package1",
                "tools/build_rules",
            },
        }
        "###);

        Ok(())
    }

    #[test]
    fn test_build_bazel_file() -> anyhow::Result<()> {
        init_logging();

        let temp = tempfile::tempdir()?;
        let fix = ScratchGitRepo::new_static_fixture(temp.path())?;

        write_files(
            &fix,
            r#"
file: WORKSPACE
load("//macro:macro.bzl", "some_macro")
some_macro()

file: package1/foo.sh

file: package1/BUILD.bazel
sh_binary(
    name = "foo",
    srcs = ["foo.sh"],
)

file: macro/BUILD

file: macro/macro.bzl
def some_macro():
    pass
"#,
        )?;
        let head_oid = fix.commit_all("Wrote files")?;
        let repo = fix.repo()?;

        let app = Arc::new(App::new_for_testing()?);
        let cache_dir = tempfile::tempdir()?;
        let resolver = BazelResolver::new(cache_dir.path());
        let target_set = hashset! {"bazel://package1:foo".try_into()?};
        let request = ResolutionRequest {
            repo: fix.path().to_path_buf(),
            targets: target_set,
        };
        let cache_options = CacheOptions::default();
        let resolve_result = resolver.resolve(&request, &cache_options, app)?;

        let odb = HashMapOdb::new();
        let files_to_materialize = {
            let head_commit = repo.find_commit(head_oid)?;
            let head_tree = head_commit.tree()?;
            let ctx = HashContext::new(&repo, &head_tree)?;
            update_object_database_from_resolution(&ctx, &odb, &resolve_result)?;
            get_files_to_materialize(&ctx, &odb, hashset! { parse_label("//package1:foo")? })?
        };
        insta::assert_debug_snapshot!(files_to_materialize, @r###"
        Ok {
            seen_keys: {
                BazelPackage(
                    Label("//package1:foo"),
                ),
                BazelPackage(
                    Label("//package1:foo.sh"),
                ),
            },
            paths: {
                "package1",
            },
        }
        "###);

        Ok(())
    }

    #[test]
    fn test_recursive_package_query() -> anyhow::Result<()> {
        init_logging();

        let temp = tempfile::tempdir()?;
        let fix = ScratchGitRepo::new_static_fixture(temp.path())?;

        write_files(
            &fix,
            r#"
file: WORKSPACE

file: macro/BUILD

file: macro/macro.bzl
def foo(name, srcs):
    native.sh_binary(
        name = name,
        srcs = srcs,
    )

file: package1/some/sub/package/foo.sh

file: package1/some/sub/package/BUILD
load("//macro:macro.bzl", "foo")
foo(
    name = "foo",
    srcs = ["foo.sh"],
)
"#,
        )?;
        let head_oid = fix.commit_all("Wrote files")?;
        let repo = fix.repo()?;

        let app = Arc::new(App::new_for_testing()?);
        let cache_dir = tempfile::tempdir()?;
        let resolver = BazelResolver::new(cache_dir.path());
        let target_set = hashset! {
            // Note that `//package1` itself is not a package, but
            // `//package1/...` expands to some number of subpackages anyways.
            "bazel://package1/...".try_into()?
        };
        let request = ResolutionRequest {
            repo: fix.path().to_path_buf(),
            targets: target_set,
        };
        let cache_options = CacheOptions::default();
        let resolve_result = resolver.resolve(&request, &cache_options, app.clone())?;
        insta::assert_debug_snapshot!(resolve_result, @r###"
        ResolutionResult {
            paths: {
                "package1",
                "package1/some/sub/package",
            },
            package_deps: {
                BazelPackage(
                    Label("//package1/..."),
                ): PackageInfo {
                    deps: {
                        BazelPackage(
                            Label("//package1/some/sub/package:foo"),
                        ),
                        BazelPackage(
                            Label("//package1/some/sub/package:foo.sh"),
                        ),
                    },
                },
                BazelPackage(
                    Label("//package1/some/sub/package:foo"),
                ): PackageInfo {
                    deps: {
                        BazelPackage(
                            Label("//package1/some/sub/package:foo.sh"),
                        ),
                    },
                },
                BazelPackage(
                    Label("//package1/some/sub/package:foo.sh"),
                ): PackageInfo {
                    deps: {},
                },
            },
        }
        "###);

        let odb = HashMapOdb::new();
        let files_to_materialize = {
            let head_commit = repo.find_commit(head_oid)?;
            let head_tree = head_commit.tree()?;
            let ctx = HashContext::new(&repo, &head_tree)?;
            update_object_database_from_resolution(&ctx, &odb, &resolve_result)?;
            get_files_to_materialize(&ctx, &odb, hashset! { parse_label("//package1/...")? })?
        };
        insta::assert_debug_snapshot!(files_to_materialize, @r###"
        Ok {
            seen_keys: {
                BazelPackage(
                    Label("//package1/..."),
                ),
                BazelPackage(
                    Label("//package1/some/sub/package:foo"),
                ),
                BazelPackage(
                    Label("//package1/some/sub/package:foo.sh"),
                ),
            },
            paths: {
                "package1",
                "package1/some/sub/package",
            },
        }
        "###);

        // Make a change that affects a subpackage without changing the tree entry containing the
        // package.
        write_files(
            &fix,
            r#"
file: WORKSPACE

file: macro/macro.bzl
def foo(name, srcs):
    native.sh_binary(
        name = name + "2",
        srcs = srcs,
    )
"#,
        )?;
        let head_oid = fix.commit_all("Wrote files")?;
        let files_to_materialize = {
            let head_commit = repo.find_commit(head_oid)?;
            let head_tree = head_commit.tree()?;
            let ctx = HashContext::new(&repo, &head_tree)?;
            get_files_to_materialize(&ctx, &odb, hashset! { parse_label("//package1/...")? })?
        };

        // The content hash for `//package1/...` has NOT changed since its `BUILD` file and tree
        // entry have remained the same, so it doesn't appear in `missing_keys` below.
        //
        // However, when we traverse its dependencies, we get to `//package1/some/sub/package:foo`
        // and try to content-hash that. Since that package's `BUILD` file has a `load` statement
        // for a package which *has* changed, that package's content hash also changes and no longer
        // matches.
        insta::assert_debug_snapshot!(files_to_materialize, @r###"
        MissingKeys {
            missing_keys: {
                (
                    BazelPackage(
                        Label("//package1/some/sub/package:foo"),
                    ),
                    ContentHash(
                        b326065aa1cb441a90f10d302dfa3af954c3a45f,
                    ),
                ),
                (
                    BazelPackage(
                        Label("//package1/some/sub/package:foo.sh"),
                    ),
                    ContentHash(
                        ec800f4eb7f1921099c813d7fd10820322366e43,
                    ),
                ),
            },
            seen_keys: {
                BazelPackage(
                    Label("//package1/..."),
                ),
                BazelPackage(
                    Label("//package1/some/sub/package:foo"),
                ),
                BazelPackage(
                    Label("//package1/some/sub/package:foo.sh"),
                ),
            },
        }
        "###);

        // Ensure that the change in the build graph is reflected.
        let resolve_result = resolver.resolve(&request, &cache_options, app)?;
        insta::assert_debug_snapshot!(resolve_result, @r###"
        ResolutionResult {
            paths: {
                "package1",
                "package1/some/sub/package",
            },
            package_deps: {
                BazelPackage(
                    Label("//package1/..."),
                ): PackageInfo {
                    deps: {
                        BazelPackage(
                            Label("//package1/some/sub/package:foo.sh"),
                        ),
                        BazelPackage(
                            Label("//package1/some/sub/package:foo2"),
                        ),
                    },
                },
                BazelPackage(
                    Label("//package1/some/sub/package:foo.sh"),
                ): PackageInfo {
                    deps: {},
                },
                BazelPackage(
                    Label("//package1/some/sub/package:foo2"),
                ): PackageInfo {
                    deps: {
                        BazelPackage(
                            Label("//package1/some/sub/package:foo.sh"),
                        ),
                    },
                },
            },
        }
        "###);

        Ok(())
    }
}
