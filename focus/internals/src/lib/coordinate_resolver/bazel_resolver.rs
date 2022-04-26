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

use crate::coordinate::{Label, TargetName};

use super::*;

/// Resolves Bazel coordinates to paths
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
            .coordinate_set
            .underlying()
            .iter()
            .filter_map(|coordinate| {
                // TODO: Consider parameterizing depth
                match coordinate {
                    Coordinate::Bazel(label) => Some(label),
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
        let (paths, packages) = {
            let query = format!(
                "buildfiles(deps({}))",
                Self::make_bazel_set(labels.iter().copied())
            );
            let result = Self::run_bazel_query(
                app.clone(),
                request,
                &["--output=package", "--noimplicit_deps"],
                &query,
            )?;

            let mut paths = BTreeSet::new();
            let mut packages = BTreeSet::new();
            #[allow(clippy::manual_flatten)]
            for line in result.lines() {
                let path = PathBuf::from(&line);
                if !line.is_empty()
                    && !line.starts_with('@')
                    && !path.starts_with("bazel-out/")
                    && !path.starts_with("external/")
                {
                    paths.insert(path);
                    packages.insert(Label::from_str(line)?);
                }
            }
            info!("'{}' requires {} packages", &query, paths.len());
            (paths, packages)
        };

        let package_top_level_targets =
            self.extract_top_level_targets(app.clone(), request, packages)?;
        let immediate_deps =
            self.extract_immediate_dependencies(app, request, package_top_level_targets)?;

        let recursive_package_queries: BTreeSet<&Label> = labels
            .into_iter()
            .filter(|label| label.target_name == TargetName::Ellipsis)
            .collect();
        let recursive_package_query_deps =
            self.add_recursive_package_query_deps(recursive_package_queries, &immediate_deps);

        let deps = immediate_deps
            .into_iter()
            .chain(recursive_package_query_deps.into_iter())
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

        let (mut cmd, scmd) =
            SandboxCommand::new(description.clone(), Self::locate_bazel_binary(request), app)?;
        scmd.ensure_success_or_log(
            cmd.arg("query")
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

    /// Extract Bazel-compatible top-level targets for the provided packages.
    fn extract_top_level_targets(
        &self,
        app: Arc<App>,
        request: &ResolutionRequest,
        packages: BTreeSet<Label>,
    ) -> Result<BTreeSet<Label>> {
        fn make_top_level_targets_spec(package: Label) -> Label {
            Label {
                // Use the `:*` syntax to get all top-level targets and files in
                // the package. We filter out the files shortly. (We can't use
                // `:all`, which would give us only the targets, because there
                // may be real targets named `all`, and there's no way to
                // disambigudate them.)
                target_name: TargetName::Name("*".to_string()),
                ..package
            }
        }

        let query = Self::make_bazel_set(packages.into_iter().map(make_top_level_targets_spec));
        let bazel_de::Query { rules } = Self::run_bazel_query_xml(app, request, vec![], &query)?;

        // TODO: This mechanism for detecting Bazel-compatibility should be
        // configurable.
        const REQUIRED_TAG: &str = "bazel-compatible";

        let mut result = BTreeSet::new();
        for rule in rules {
            match rule {
                bazel_de::QueryElement::Rule(bazel_de::Rule {
                    name,
                    location,
                    elements,
                }) => {
                    let is_defined_in_bazel_file = match location {
                        None => false,
                        Some(location) => location.contains("/BUILD.bazel:"),
                    };
                    let has_required_tag = || {
                        elements.iter().any(|tag_element| match tag_element {
                            bazel_de::RuleElement::List(bazel_de::List { name, values })
                                if name == "tags" =>
                            {
                                values.iter().any(|value| match value {
                                    bazel_de::RuleElement::String {
                                        name: None,
                                        value: Some(value),
                                    } => value == REQUIRED_TAG,
                                    _ => false,
                                })
                            }
                            _ => false,
                        })
                    };

                    if is_defined_in_bazel_file || has_required_tag() {
                        let label: Label = name.parse()?;
                        result.insert(label);
                    }
                }

                bazel_de::QueryElement::SourceFile { name, .. }
                | bazel_de::QueryElement::GeneratedFile { name, .. } => {
                    let label: Label = name.parse()?;
                    result.insert(label);
                }

                bazel_de::QueryElement::PackageGroup { .. } => {
                    // Do not include as top-level targets.
                }
            }
        }
        Ok(result)
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
                    | DependencyKey::Path(_) => todo!(),
                    DependencyKey::DummyForTesting(_) => todo!(),
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
