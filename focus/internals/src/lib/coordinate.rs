use anyhow::Result;
use std::{collections::HashSet, convert::TryFrom, fmt::Display};

use thiserror::Error;

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct CoordinateSet {
    underlying: HashSet<Coordinate>,
    uniform: bool,
}

impl CoordinateSet {
    pub fn underlying(&self) -> &HashSet<Coordinate> {
        &self.underlying
    }

    pub fn is_uniform(&self) -> bool {
        self.uniform
    }

    pub fn determine_uniformity(set: &HashSet<Coordinate>) -> bool {
        let mut count_by_type = [0_usize; 3];

        for coordinate in set {
            match coordinate {
                Coordinate::Bazel(_) => count_by_type[0] += 1,
                Coordinate::Directory(_) => count_by_type[1] += 1,
                Coordinate::Pants(_) => count_by_type[2] += 1,
            }
        }

        let distinct_types_in_counts = count_by_type.into_iter().filter(|count| *count > 0).count();
        distinct_types_in_counts < 2
    }
}

impl From<HashSet<Coordinate>> for CoordinateSet {
    fn from(underlying: HashSet<Coordinate>) -> Self {
        let uniform = Self::determine_uniformity(&underlying);
        Self {
            underlying,
            uniform,
        }
    }
}

impl TryFrom<&[String]> for CoordinateSet {
    type Error = CoordinateError;

    fn try_from(coordinates: &[String]) -> Result<Self, Self::Error> {
        let mut underlying = HashSet::<Coordinate>::new();

        for coordinate in coordinates {
            match Coordinate::try_from(coordinate.as_str()) {
                Ok(coordinate) => {
                    underlying.insert(coordinate);
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

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub enum Coordinate {
    Bazel(String),
    Directory(String),
    Pants(String),
}

impl Display for Coordinate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Coordinate::Bazel(c) => write!(f, "{}", c),
            Coordinate::Directory(c) => write!(f, "{}", c),
            Coordinate::Pants(c) => write!(f, "{}", c),
        }
    }
}

#[derive(Error, Debug, PartialEq)]
pub enum CoordinateError {
    #[error("Scheme not supported")]
    UnsupportedScheme(String),

    #[error("Failed to tokenize input")]
    TokenizationError,
}

impl TryFrom<&str> for Coordinate {
    type Error = CoordinateError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.split_once(":") {
            Some((prefix, rest)) => {
                let rest = rest.to_owned();
                if prefix.eq_ignore_ascii_case("bazel") {
                    Ok(Coordinate::Bazel(rest))
                } else if prefix.eq_ignore_ascii_case("directory") {
                    Ok(Coordinate::Directory(rest))
                } else if prefix.eq_ignore_ascii_case("pants") {
                    Ok(Coordinate::Pants(rest))
                } else {
                    Err(CoordinateError::UnsupportedScheme(prefix.to_owned()))
                }
            }
            None => Err(CoordinateError::TokenizationError),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use anyhow::Result;

    use crate::coordinate::{Coordinate, CoordinateError, CoordinateSet};

    #[test]
    pub fn coordinate_parsing() -> Result<()> {
        assert_eq!(
            Coordinate::try_from("bazel://a:b")?,
            Coordinate::Bazel("//a:b".to_owned())
        );
        assert_eq!(
            Coordinate::try_from("bogus:whatever").unwrap_err(),
            CoordinateError::UnsupportedScheme("bogus".to_owned())
        );
        assert_eq!(
            Coordinate::try_from("okay").unwrap_err(),
            CoordinateError::TokenizationError
        );

        Ok(())
    }

    #[test]
    pub fn sets_from_strings_of_coordinates() -> Result<()> {
        let coordinates = vec![String::from("bazel://a:b"), String::from("bazel://x/y:z")];

        let set = CoordinateSet::try_from(coordinates.as_slice());
        let set = set.unwrap();
        assert_eq!(set.underlying().len(), 2);
        assert!(set.is_uniform());
        Ok(())
    }

    // TODO: Enable this again when there are more coordinate types.
    // #[cfg(disabled_test)]
    #[test]
    pub fn non_uniform_sets() -> Result<()> {
        // Sets containing different coordinate types are non-uniform
        assert!(!CoordinateSet::try_from(&[
            String::from("bazel://a:b"),
            String::from("directory:/foo"),
        ] as &[String])?
        .is_uniform());

        // Empty sets are uniform
        assert!(CoordinateSet::try_from(&[] as &[String])?.is_uniform());

        Ok(())
    }

    #[test]
    pub fn failed_conversion_of_sets() -> Result<()> {
        assert_eq!(
            CoordinateSet::try_from(&[String::from("whatever")] as &[String]).unwrap_err(),
            CoordinateError::TokenizationError
        );
        assert_eq!(
            CoordinateSet::try_from(&[String::from("foo:bar")] as &[String]).unwrap_err(),
            CoordinateError::UnsupportedScheme("foo".to_owned())
        );

        Ok(())
    }
}
