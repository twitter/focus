// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{
    borrow::Borrow,
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Mutex,
};

use anyhow::{bail, Result};
use focus_util::sandbox_command::{SandboxCommand, SandboxCommandOutput};
use tracing::{debug, info};

use crate::target::{Label, TargetName};

use super::*;

const OUTLINING_BAZELRC_PATH: &str = "focus/outlining.bazelrc";

/// Resolves Bazel targets to paths
pub struct BazelResolver {
    #[allow(dead_code)]
    cache_root: PathBuf,

    mutex: Mutex<()>,
}

impl BazelResolver {
    fn locate_bazel_binary(request: &ResolutionRequest) -> PathBuf {
        let in_repo_bazel_wrapper = request.repo.join("bazel");
        if in_repo_bazel_wrapper.is_file() {
            // This is dumb, but our wrapper script pukes if you invoke it with an absolute path. We are just ensuring that it exists at all.
            PathBuf::from("./bazel")
        } else {
            PathBuf::from("bazel")
        }
    }
}

impl Resolver for BazelResolver {
    fn new(cache_root: &Path) -> Self {
        Self {
            cache_root: cache_root.join("bazel"),
            mutex: Mutex::new(()),
        }
    }

    fn resolve(
        &self,
        request: &ResolutionRequest,
        _cache_options: &CacheOptions,
        app: Arc<App>,
    ) -> Result<ResolutionResult> {
        let lock = self.mutex.lock();
        if let Err(e) = lock {
            bail!("Failed to lock mutex: {}", e);
        }

        let mut directories = BTreeSet::<PathBuf>::new();
        let mut package_deps = BTreeMap::new();
        let labels: HashSet<&Label> = request
            .targets
            .iter()
            .filter_map(|target| {
                // TODO: Consider parameterizing depth
                match target {
                    Target::Bazel(label) => Some(label),
                    _ => unreachable!(),
                }
            })
            .collect();

        #[allow(clippy::redundant_clone)]
        let app = app.clone();

        let (paths, deps) = self.query_package_dependencies(app, request, labels)?;
        directories.extend(paths);
        package_deps.extend(deps);

        Ok(ResolutionResult {
            paths: directories,
            package_deps,
        })
    }
}

impl BazelResolver {
    fn query_package_dependencies(
        &self,
        app: Arc<App>,
        request: &ResolutionRequest,
        labels: HashSet<&Label>,
    ) -> anyhow::Result<(BTreeSet<PathBuf>, BTreeMap<DependencyKey, DependencyValue>)> {
        let dep_labels = {
            let query = format!(
                // Use `deps(...)` so that we preserve the actual names of the
                // targets which were declared as dependencies, but also add in
                // any `buildfiles` dependencies that might exist in the
                // repository. This includes dependencies on `BUILD` or `.bzl`
                // files (such as those `load`ed by the other `BUILD` files).
                //
                // We limit buildfile dependencies to only those in the
                // repository, because `.bzl` files, etc., in external
                // repositories are typically not supported, so querying them
                // fails.
                "deps({0}) union (buildfiles(deps({0})) intersect //...)",
                Self::make_bazel_set(labels.iter().copied())
            );

            let result =
                Self::run_bazel_query(app.clone(), request, &["--noimplicit_deps"], &query)?;

            // Initialize `labels` to the set of labels that we were given.
            // It's possible that those labels will not appear in the
            // `buildfiles(deps(...))` output if the only dependencies for those
            // labels are in external repositories, so we need to make sure to
            // include them explicitly.
            let mut dep_labels: BTreeSet<Label> = labels.iter().copied().cloned().collect();

            for line in result.lines() {
                dep_labels.insert(Label::from_str(line)?);
            }

            info!("'{}' requires {} packages", &query, dep_labels.len());
            dep_labels
        };

        let immediate_deps = self.extract_immediate_dependencies(app, request, dep_labels)?;

        let recursive_package_queries: BTreeSet<&Label> = labels
            .into_iter()
            .filter(|label| label.target_name == TargetName::Ellipsis)
            .collect();
        let recursive_package_query_deps =
            self.add_recursive_package_query_deps(recursive_package_queries, &immediate_deps);

        let immediate_keys: HashSet<_> = immediate_deps.keys().collect();
        let recursive_keys: HashSet<_> = recursive_package_query_deps.keys().collect();
        let intersection: HashSet<_> = immediate_keys.intersection(&recursive_keys).collect();
        assert!(intersection.is_empty());

        let deps: BTreeMap<DependencyKey, DependencyValue> = immediate_deps
            .into_iter()
            .chain(recursive_package_query_deps.into_iter())
            .collect();

        let paths = deps
            .iter()
            .filter_map(|(dep_key, _dep_value)| match dep_key {
                DependencyKey::BazelPackage(Label {
                    external_repository: None,
                    path_components,
                    target_name: _,
                })
                | DependencyKey::BazelBuildFile(Label {
                    external_repository: None,
                    path_components,
                    target_name: _,
                }) => {
                    let path: PathBuf = path_components.iter().collect();
                    Some(path)
                }

                DependencyKey::BazelPackage(Label {
                    external_repository: Some(_),
                    path_components: _,
                    target_name: _,
                })
                | DependencyKey::BazelBuildFile(Label {
                    external_repository: Some(_),
                    path_components: _,
                    target_name: _,
                }) => {
                    // Don't need to materialize external repositories.
                    None
                }

                DependencyKey::Path(path) => Some(path.clone()),

                key @ DependencyKey::DummyForTesting(_) => {
                    panic!("Got dummy dependency key: {:?}", key)
                }
            })
            .collect();

        Ok((paths, deps))
    }

    fn run_bazel_query(
        app: Arc<App>,
        request: &ResolutionRequest,
        bazel_args: &[&str],
        query: &str,
    ) -> Result<String> {
        let description = format!("bazel query '{}'", query);

        let query_file_path = {
            let (mut file, path, _serial) = app
                .sandbox()
                .create_file(Some("bazel_query"), None, None)
                .context("creating bazel query file")?;
            file.write_all(query.as_bytes())
                .context("writing bazel query to disk")?;
            path
        };

        let mut initial_bazel_args = Vec::<String>::new();
        if request.repo.join(OUTLINING_BAZELRC_PATH).is_file() {
            initial_bazel_args.push(String::from("--noworkspace_rc"));
            initial_bazel_args.push(format!("--bazelrc={}", OUTLINING_BAZELRC_PATH));
        }
        let (mut cmd, scmd) =
            SandboxCommand::new(description.clone(), Self::locate_bazel_binary(request), app)?;
        scmd.ensure_success_or_log(
            cmd.args(initial_bazel_args)
                .arg("query")
                .arg("--query_file")
                .arg(query_file_path)
                .args(bazel_args)
                .current_dir(&request.repo),
            SandboxCommandOutput::Stderr,
            &description,
        )?;

        // Read to string so that we can print it if we need to debug.
        let raw_result = {
            let mut result = String::new();
            scmd.read_to_string(SandboxCommandOutput::Stdout, &mut result)?;
            result
        };
        debug!(?query, ?raw_result, "Query returned with result");
        Ok(raw_result)
    }

    fn run_bazel_query_xml(
        app: Arc<App>,
        request: &ResolutionRequest,
        bazel_args: Vec<&str>,
        query: &str,
    ) -> Result<bazel_de::Query> {
        let bazel_args = {
            let mut bazel_args = bazel_args;
            bazel_args.push("--output=xml");
            bazel_args
        };
        let raw_result = Self::run_bazel_query(app, request, &bazel_args, query)?;
        let parsed_result: bazel_de::Query = serde_xml_rs::from_str(&raw_result)?;
        Ok(parsed_result)
    }

    fn make_bazel_set(labels: impl IntoIterator<Item = impl Borrow<Label>>) -> String {
        format!(
            "set({})",
            labels
                .into_iter()
                .map(|label| label.borrow().to_string())
                .map(|label| Self::quote_target_name(&label))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }

    /// Escape any characters with special meaning to Bazel. For example, by
    /// default, Bazel will try to lex curly braces (`{}`) as part of a
    /// different token.
    fn quote_target_name(target_name: &str) -> String {
        format!("\"{}\"", target_name)
    }

    /// Calculate the immediate dependencies of the provided targets.
    fn extract_immediate_dependencies(
        &self,
        app: Arc<App>,
        request: &ResolutionRequest,
        targets: BTreeSet<Label>,
    ) -> Result<BTreeMap<DependencyKey, DependencyValue>> {
        let query = Self::make_bazel_set(targets.iter());
        let bazel_de::Query { rules } =
            Self::run_bazel_query_xml(app, request, vec!["--noimplicit_deps"], &query)?;

        let mut immediate_deps: BTreeMap<DependencyKey, BTreeSet<DependencyKey>> = BTreeMap::new();
        for rule in rules {
            use bazel_de::{QueryElement, Rule, RuleElement};

            match rule {
                QueryElement::Rule(Rule {
                    name,
                    location: _,
                    elements,
                }) => {
                    let target_label: Label = name.parse()?;

                    let mut rule_inputs = BTreeSet::new();
                    for rule_element in elements {
                        match rule_element {
                            RuleElement::RuleInput { name } => {
                                let label: Label = name.parse()?;
                                rule_inputs.insert(DependencyKey::BazelPackage(label));
                            }

                            RuleElement::Boolean { .. }
                            | RuleElement::Int { .. }
                            | RuleElement::String { .. }
                            | RuleElement::List { .. }
                            | RuleElement::Dict { .. }
                            | RuleElement::Label(_)
                            | RuleElement::VisibilityLabel { .. }
                            | RuleElement::RuleOutput { .. }
                            | RuleElement::Output { .. }
                            | RuleElement::Tristate { .. } => {
                                // Do nothing.
                            }
                        }
                    }

                    let key = DependencyKey::BazelPackage(target_label);
                    immediate_deps.entry(key).or_default().extend(rule_inputs);
                }

                QueryElement::SourceFile { name, body: () }
                | QueryElement::GeneratedFile { name, body: () } => {
                    let target_label: Label = name.parse()?;
                    let key = DependencyKey::BazelPackage(target_label);

                    // Create an entry for this package if it doesn't exist. We
                    // don't need to add any dependencies, since source files
                    // inside the package will already be checked out as part of
                    // materializing the package itself, and generated files will be generated by whatever rule generates them.
                    immediate_deps.entry(key.clone()).or_default();
                }

                QueryElement::PackageGroup { body: () } => {
                    // TODO: do we need to do anything for these cases?
                }
            }
        }

        let result: BTreeMap<DependencyKey, DependencyValue> = immediate_deps
            .into_iter()
            .map(|(k, v)| (k, DependencyValue::PackageInfo { deps: v }))
            .collect();
        Ok(result)
    }

    /// The result returned by Bazel for the dependencies of `//foo/...` does
    /// not include an entry for the label `//foo/...` itself. We synthesize
    /// our own dependencies for a term like `//foo/...` by manually
    /// traversing the results and associating e.g. `//foo/bar:baz` as a
    /// dependency for the synthetic target `//foo/...`.
    ///
    /// Note that `//foo` itself may not be a package! But `//foo/...` will
    /// still expand to e.g. `//foo/bar:baz + //foo/qux/xyzzy:plugh`, for
    /// which `//foo/bar` and `//foo/qux/xyzyy` *are* packages.
    ///
    /// NB: This is an O(n^2) algorithm. This could be O(n), but for now,
    /// we're expecting a relatively small number of (hand-written) labels of
    /// the form `//foo/...`
    fn add_recursive_package_query_deps(
        &self,
        recursive_package_queries: BTreeSet<&Label>,
        immediate_deps: &BTreeMap<DependencyKey, DependencyValue>,
    ) -> BTreeMap<DependencyKey, DependencyValue> {
        let mut deps: BTreeMap<Label, BTreeSet<DependencyKey>> = BTreeMap::new();
        for containing_target in recursive_package_queries {
            debug_assert_eq!(containing_target.target_name, TargetName::Ellipsis);

            // Given the label `//foo/...`, try to find targets like
            // `//foo/bar:baz`, for which `//foo` is a prefix.
            for dep_key in immediate_deps.keys() {
                match dep_key {
                    DependencyKey::BazelPackage(Label {
                        external_repository: None,
                        path_components,
                        target_name,
                    })
                    | DependencyKey::BazelBuildFile(Label {
                        external_repository: None,
                        path_components,
                        target_name,
                    }) => {
                        debug_assert_ne!(target_name, &TargetName::Ellipsis);

                        if path_components.starts_with(&containing_target.path_components) {
                            deps.entry((*containing_target).clone())
                                .or_default()
                                .insert(dep_key.clone());
                        }
                    }

                    DependencyKey::BazelPackage(Label {
                        external_repository: Some(_),
                        path_components: _,
                        target_name: _,
                    })
                    | DependencyKey::BazelBuildFile(Label {
                        external_repository: Some(_),
                        path_components: _,
                        target_name: _,
                    })
                    | DependencyKey::Path(_) => {
                        // None of these could have been associated with a
                        // `//...` pattern inside the repository itself.
                    }

                    key @ DependencyKey::DummyForTesting(_) => {
                        panic!("Got dummy dependency key: {:?}", key);
                    }
                }
            }
        }

        deps.into_iter()
            .map(|(k, v)| {
                (
                    DependencyKey::BazelPackage(k),
                    DependencyValue::PackageInfo { deps: v },
                )
            })
            .collect()
    }
}
