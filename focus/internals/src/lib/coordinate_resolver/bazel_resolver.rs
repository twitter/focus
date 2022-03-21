use std::{
    io::BufRead,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Mutex,
};

use anyhow::{bail, Result};
use tracing::{debug, info};

use crate::{
    coordinate::{Label, TargetName},
    util::sandbox_command::{SandboxCommand, SandboxCommandOutput},
};

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

        let deps = self.extract_dependencies(app, request, packages)?;
        Ok((paths, deps))
    }

    fn extract_dependencies(
        &self,
        app: Arc<App>,
        request: &ResolutionRequest,
        packages: BTreeSet<Label>,
    ) -> Result<BTreeMap<DependencyKey, DependencyValue>> {
        const REQUIRED_TAG: &str = "bazel-compatible";

        let query = format!(
            "deps(attr('tags', '{}', set({})))",
            REQUIRED_TAG,
            packages
                .into_iter()
                .map(|package| format!(
                    "{}",
                    Label {
                        target_name: TargetName::Name("all".to_string()),
                        ..package
                    }
                ))
                .collect::<Vec<_>>()
                .join(" "),
        );
        let description = format!("bazel query '{}'", query);

        let (mut cmd, scmd) =
            SandboxCommand::new(description.clone(), Self::locate_bazel_binary(request), app)?;
        scmd.ensure_success_or_log(
            cmd.arg("query")
                .arg(&query)
                .arg("--output=xml")
                .arg("--noimplicit_deps")
                .current_dir(&request.repo),
            SandboxCommandOutput::Stderr,
            &description,
        )?;

        // Read to string so that we can print it if we need to debug.
        let query_result = {
            let mut result = String::new();
            scmd.read_to_string(SandboxCommandOutput::Stdout, &mut result)?;
            result
        };
        let bazel_de::Query { rules } = serde_xml_rs::from_str(&query_result)?;
        debug!(?query, ?query_result, "Query returned with result");

        let mut result: BTreeMap<DependencyKey, BTreeSet<DependencyKey>> = BTreeMap::new();
        for rule in rules {
            use bazel_de::{QueryElement, Rule, RuleElement};

            match rule {
                QueryElement::Rule(Rule { name, elements }) => {
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
