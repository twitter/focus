use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::str::FromStr;
use std::{collections::HashSet, convert::TryFrom, fmt::Display};

use thiserror::Error;

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct TargetSet {
    underlying: HashSet<Target>,
    uniform: bool,
}

impl TargetSet {
    pub fn underlying(&self) -> &HashSet<Target> {
        &self.underlying
    }

    pub fn is_uniform(&self) -> bool {
        self.uniform
    }

    pub fn determine_uniformity(set: &HashSet<Target>) -> bool {
        let mut count_by_type = [0_usize; 3];

        for target in set {
            match target {
                Target::Bazel(_) => count_by_type[0] += 1,
                Target::Directory(_) => count_by_type[1] += 1,
                Target::Pants(_) => count_by_type[2] += 1,
            }
        }

        let distinct_types_in_counts = count_by_type.into_iter().filter(|count| *count > 0).count();
        distinct_types_in_counts < 2
    }
}

impl From<HashSet<Target>> for TargetSet {
    fn from(underlying: HashSet<Target>) -> Self {
        let uniform = Self::determine_uniformity(&underlying);
        Self {
            underlying,
            uniform,
        }
    }
}

impl TryFrom<&[String]> for TargetSet {
    type Error = TargetError;

    fn try_from(targets: &[String]) -> Result<Self, Self::Error> {
        let mut underlying = HashSet::<Target>::new();

        for target in targets {
            match Target::try_from(target.as_str()) {
                Ok(target) => {
                    underlying.insert(target);
                }
                Err(e) => return Err(e),
            }
        }

        let uniform = Self::determine_uniformity(&underlying);
        Ok(Self {
            underlying,
            uniform,
        })
    }
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Clone)]
pub enum Target {
    /// A Bazel package like `//foo/bar:baz`.
    Bazel(Label),

    /// A specific directory within the repository.
    Directory(String),

    /// A Pants package like `foo/bar:baz`.
    Pants(String),
}

impl Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Bazel(c) => write!(f, "{}", c),
            Target::Directory(c) => write!(f, "{}", c),
            Target::Pants(c) => write!(f, "{}", c),
        }
    }
}

#[derive(Error, Debug, PartialEq)]
pub enum TargetError {
    #[error("Scheme not supported")]
    UnsupportedScheme(String),

    #[error("No target scheme provided")]
    NoSchemeProvidedError,

    #[error("Failed to parse label")]
    LabelError(#[from] LabelParseError),
}

impl TryFrom<&str> for Target {
    type Error = TargetError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.split_once(':') {
            Some((prefix, rest)) => {
                let rest = rest.to_owned();
                if prefix.eq_ignore_ascii_case("bazel") {
                    let label: Label = rest.parse()?;
                    Ok(Target::Bazel(label))
                } else if prefix.eq_ignore_ascii_case("directory") {
                    Ok(Target::Directory(rest))
                } else if prefix.eq_ignore_ascii_case("pants") {
                    Ok(Target::Pants(rest))
                } else {
                    Err(TargetError::UnsupportedScheme(prefix.to_owned()))
                }
            }
            None => Err(TargetError::NoSchemeProvidedError),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TargetName {
    Name(String),
    Ellipsis,
}

/// A Bazel label referring to a specific target.
///
/// See <https://docs.bazel.build/versions/main/build-ref.html#labels>. Note
/// that a label does *not* refer to a package.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Label {
    /// For a label like `@foo//bar:baz`, this would be `@foo`. If there is no
    /// `@`-component, then this is `None`.
    pub(crate) external_repository: Option<String>,

    /// The directory components of the path after `//`.
    ///
    /// The leading `//` is optional and inferred if present (i.e. a label
    /// `foo/bar` is assumed to be the same as `//foo/bar`, and not instead
    /// relative to the current directory.)
    pub(crate) path_components: Vec<String>,

    /// If no explicit target name is given, it is inferred from the last path
    /// component. For a label like `//foo/bar:bar` or `//foo/bar`, this would
    /// be `bar`.
    pub(crate) target_name: TargetName,
}

impl Display for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}//{}",
            self.external_repository.as_deref().unwrap_or_default(),
            // Note that `path_components` may be empty, which is fine.
            self.path_components.join("/")
        )?;

        match &self.target_name {
            TargetName::Name(name) => {
                write!(f, ":{}", name)?;
            }
            TargetName::Ellipsis => {
                write!(f, "/...")?;
            }
        }

        Ok(())
    }
}

impl Debug for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label_string: String = format!("{}", self);
        write!(f, r#"Label({:?})"#, label_string)
    }
}

/// TODO: improve error messaging here
#[derive(Error, Debug, PartialEq)]
pub enum LabelParseError {
    #[error("No target name")]
    NoTargetName,

    #[error("Empty label")]
    EmptyLabel,
}

impl FromStr for Label {
    type Err = LabelParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (external_package, label) = match s.split_once("//") {
            None => (None, s),
            Some(("", label)) => (None, label),
            Some((external_package, label)) => (Some(external_package.to_string()), label),
        };

        let (package, target) = match label.split_once(':') {
            Some((package, target)) => (package, Some(target)),
            None => (label, None),
        };

        let path_components: Vec<String> = package.split('/').map(|s| s.to_string()).collect();
        let target = match (path_components.last(), target) {
            (Some(_last_component), Some(target)) => target.to_string(),
            (None, Some(target)) => target.to_string(),
            (Some(last_component), None) if last_component.is_empty() => {
                return Err(LabelParseError::EmptyLabel)
            }
            (None, None) => return Err(LabelParseError::EmptyLabel),
            (Some(last_component), None) => last_component.clone(),
        };

        if target == "..." {
            let mut path_components = path_components;
            path_components.pop();
            Ok(Self {
                external_repository: external_package,
                path_components,
                target_name: TargetName::Ellipsis,
            })
        } else {
            Ok(Self {
                external_repository: external_package,
                path_components,
                target_name: TargetName::Name(target),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use anyhow::Result;

    use super::*;

    #[test]
    pub fn coordinate_parsing() -> Result<()> {
        assert_eq!(
            Target::try_from("bazel://a:b")?,
            Target::Bazel(Label {
                external_repository: None,
                path_components: vec!["a".to_string()],
                target_name: TargetName::Name("b".to_string()),
            })
        );
        assert_eq!(
            Target::try_from("bazel://foo"),
            Ok(Target::Bazel(Label {
                external_repository: None,
                path_components: vec!["foo".to_string()],
                target_name: TargetName::Name("foo".to_string())
            }))
        );
        assert_eq!(
            Target::try_from("bazel://foo/bar/..."),
            Ok(Target::Bazel(Label {
                external_repository: None,
                path_components: vec!["foo".to_string(), "bar".to_string()],
                target_name: TargetName::Ellipsis,
            }))
        );
        assert_eq!(
            Target::try_from("bazel:@foo//bar:qux"),
            Ok(Target::Bazel(Label {
                external_repository: Some("@foo".to_string()),
                path_components: vec!["bar".to_string()],
                target_name: TargetName::Name("qux".to_string()),
            }))
        );
        assert_eq!(
            Target::try_from("bazel://foo/bar:baz/qux.py"),
            Ok(Target::Bazel(Label {
                external_repository: None,
                path_components: vec!["foo".to_string(), "bar".to_string()],
                target_name: TargetName::Name("baz/qux.py".to_string()),
            }))
        );

        assert_eq!(
            Target::try_from("bogus:whatever").unwrap_err(),
            TargetError::UnsupportedScheme("bogus".to_owned())
        );
        assert_eq!(
            Target::try_from("okay").unwrap_err(),
            TargetError::NoSchemeProvidedError
        );
        assert_eq!(
            Target::try_from("bazel://").unwrap_err(),
            TargetError::LabelError(LabelParseError::EmptyLabel),
        );

        Ok(())
    }

    #[test]
    pub fn sets_from_strings_of_coordinates() -> Result<()> {
        let targets = vec![String::from("bazel://a:b"), String::from("bazel://x/y:z")];

        let set = TargetSet::try_from(targets.as_slice());
        let set = set.unwrap();
        assert_eq!(set.underlying().len(), 2);
        assert!(set.is_uniform());
        Ok(())
    }

    // TODO: Enable this again when there are more target types.
    // #[cfg(disabled_test)]
    #[test]
    pub fn non_uniform_sets() -> Result<()> {
        // Sets containing different target types are non-uniform
        assert!(!TargetSet::try_from(
            &[String::from("bazel://a:b"), String::from("directory:/foo"),] as &[String]
        )?
        .is_uniform());

        // Empty sets are uniform
        assert!(TargetSet::try_from(&[] as &[String])?.is_uniform());

        Ok(())
    }

    #[test]
    pub fn failed_conversion_of_sets() -> Result<()> {
        assert_eq!(
            TargetSet::try_from(&[String::from("whatever")] as &[String]).unwrap_err(),
            TargetError::NoSchemeProvidedError
        );
        assert_eq!(
            TargetSet::try_from(&[String::from("foo:bar")] as &[String]).unwrap_err(),
            TargetError::UnsupportedScheme("foo".to_owned())
        );

        Ok(())
    }
}
