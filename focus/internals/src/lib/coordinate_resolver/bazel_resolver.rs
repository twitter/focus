use std::{
    io::BufRead,
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
        let labels: Vec<&Label> = request
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

        for label in labels {
            let (paths, deps) = self.query_package_dependencies(app.clone(), request, label)?;
            directories.extend(paths);
            package_deps.extend(deps);
        }

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
        label: &Label,
    ) -> anyhow::Result<(BTreeSet<PathBuf>, BTreeMap<DependencyKey, DependencyValue>)> {
        let mut paths = BTreeSet::new();
        let mut packages = BTreeSet::new();
        let query = format!("buildfiles(deps({}))", label);

        // Run Bazel query
        let description = format!("bazel query '{}'", &query);
        let (mut cmd, scmd) = SandboxCommand::new(
            description.clone(),
            Self::locate_bazel_binary(request),
            app.clone(),
        )?;
        scmd.ensure_success_or_log(
            cmd.arg("query")
                .arg(&query)
                .arg("--output=package")
                .arg("--noimplicit_deps")
                .current_dir(&request.repo),
            SandboxCommandOutput::Stderr,
            &description,
        )?;

        let reader = scmd.read_buffered(SandboxCommandOutput::Stdout)?;
        #[allow(clippy::manual_flatten)]
        for line in reader.lines() {
            if let Ok(line) = line {
                let path = PathBuf::from(&line);
                if !&line.starts_with('@')
                    && !path.starts_with("bazel-out/")
                    && !path.starts_with("external/")
                {
                    paths.insert(path);
                    packages.insert(Label::from_str(&line)?);
                }
            }
        }
        info!("'{}' requires {} directories", &query, paths.len(),);

        // Avoid exceeding max argument list length.
        const MAX_NUM_ARGS: usize = 1000;

        let targets = self.extract_targets(app.clone(), request, packages)?;
        let deps = {
            let mut result = Vec::new();
            for chunk in targets
                .into_iter()
                .collect::<Vec<_>>()
                .as_slice()
                .chunks(MAX_NUM_ARGS)
            {
                result.extend(self.extract_dependencies(
                    app.clone(),
                    request,
                    chunk.into_iter().cloned().collect(),
                )?);
            }
            result.into_iter().collect()
        };
        Ok((paths, deps))
    }

    fn run_bazel_query(
        app: Arc<App>,
        request: &ResolutionRequest,
        bazel_args: &[&str],
        query: &str,
    ) -> Result<bazel_de::Query> {
        let description = format!("bazel query '{}'", query);

        let (mut cmd, scmd) =
            SandboxCommand::new(description.clone(), Self::locate_bazel_binary(request), app)?;
        scmd.ensure_success_or_log(
            cmd.arg("query")
                .arg(&query)
                .arg("--output=xml")
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
        let parsed_result: bazel_de::Query = serde_xml_rs::from_str(&raw_result)?;
        Ok(parsed_result)
    }

    /// Extract Bazel-compatible top-level targets for the provided packages.
    fn extract_targets(
        &self,
        app: Arc<App>,
        request: &ResolutionRequest,
        packages: BTreeSet<Label>,
    ) -> Result<BTreeSet<Label>> {
        fn make_top_level_targets_spec(package: Label) -> String {
            let spec = Label {
                // Use the `:*` syntax to get all top-level targets and files in
                // the package. We filter out the files shortly. (We can't use
                // `:all`, which would give us only the targets, because there
                // may be real targets named `all`, and there's no way to
                // disambigudate them.)
                target_name: TargetName::Name("*".to_string()),
                ..package
            };
            spec.to_string()
        }

        let query = format!(
            "set({})",
            packages
                .into_iter()
                .map(make_top_level_targets_spec)
                .collect::<Vec<_>>()
                .join(" ")
        );
        let bazel_de::Query { rules } = Self::run_bazel_query(app, request, &[], &query)?;

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

                bazel_de::QueryElement::SourceFile { .. }
                | bazel_de::QueryElement::GeneratedFile { .. }
                | bazel_de::QueryElement::PackageGroup { .. } => {
                    // Do not include as top-level targets.
                }
            }
        }
        Ok(result)
    }

    /// Calculate the transitive dependencies of the provided targets.
    fn extract_dependencies(
        &self,
        app: Arc<App>,
        request: &ResolutionRequest,
        targets: BTreeSet<Label>,
    ) -> Result<BTreeMap<DependencyKey, DependencyValue>> {
        let query = format!(
            "deps(set({}))",
            targets
                .into_iter()
                .map(|target| target.to_string())
                .collect::<Vec<_>>()
                .join(" "),
        );
        let bazel_de::Query { rules } =
            Self::run_bazel_query(app, request, &["--noimplicit_deps"], &query)?;

        let mut result: BTreeMap<DependencyKey, BTreeSet<DependencyKey>> = BTreeMap::new();
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
                                rule_inputs.insert(DependencyKey::new_bazel_package(label));
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

                    let key = DependencyKey::new_bazel_package(target_label);
                    result.entry(key).or_default().extend(rule_inputs);
                }

                QueryElement::SourceFile { name, body: () } => {
                    let target_label: Label = name.parse()?;
                    let key = DependencyKey::new_bazel_package(target_label);

                    // Create an entry for this package if it doesn't exist. We
                    // don't need to add any dependencies, since source files
                    // inside the package will already be checked out as part of
                    // materializing the package itself.
                    result.entry(key.clone()).or_default();
                }

                QueryElement::GeneratedFile { body: () }
                | QueryElement::PackageGroup { body: () } => {
                    // TODO: do we need to do anything for these cases?
                }
            }
        }

        let result = result
            .into_iter()
            .map(|(k, v)| (k, DependencyValue::PackageInfo { deps: v }))
            .collect();
        Ok(result)
    }
}
