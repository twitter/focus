// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use std::{
    cell::RefCell,
    cmp::Ordering,
    collections::{BTreeSet, HashSet},
    ffi::OsString,
    fs::File,
    io::Write,
    os::unix::prelude::{OsStrExt, OsStringExt},
    path::{Path, PathBuf, MAIN_SEPARATOR},
    str::FromStr,
};

use lazy_static::lazy_static;

const MAIN_SEPARATOR_BYTE: u8 = MAIN_SEPARATOR as u8;
const MAIN_SEPARATOR_BYTES: &[u8] = &[MAIN_SEPARATOR_BYTE];

#[derive(Clone, Debug, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "pattern")]
pub enum Pattern {
    Verbatim {
        #[serde(default = "pattern_default_precedence")]
        precedence: usize,
        fragment: String,
    },
    Directory {
        #[serde(default = "pattern_default_precedence")]
        precedence: usize,
        path: std::path::PathBuf,
        recursive: bool,
    },
}

fn pattern_default_precedence() -> usize {
    usize::MAX
}

impl PartialOrd for Pattern {
    /// Verbatim patterns always precede Directory patterns. Either are kept in order.
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (
                Pattern::Verbatim {
                    precedence: i0,
                    fragment: s0,
                },
                Pattern::Verbatim {
                    precedence: i1,
                    fragment: s1,
                },
            ) => match i0.partial_cmp(i1) {
                Some(Ordering::Equal) => s0.partial_cmp(s1),
                Some(nonequal_ordering) => Some(nonequal_ordering),
                None => None,
            },
            (Pattern::Verbatim { .. }, Pattern::Directory { .. }) => Some(Ordering::Greater),
            (Pattern::Directory { .. }, Pattern::Verbatim { .. }) => Some(Ordering::Less),
            (
                Pattern::Directory {
                    precedence: i0,
                    path: p0,
                    recursive: _r0,
                },
                Pattern::Directory {
                    precedence: i1,
                    path: p1,
                    recursive: _r1,
                },
            ) => match i0.partial_cmp(i1) {
                Some(Ordering::Equal) => p0.partial_cmp(p1),
                Some(nonequal_ordering) => Some(nonequal_ordering),
                None => None,
            },
        }
    }
}

impl Ord for Pattern {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap()
    }
}

impl PartialEq for Pattern {
    /// Equality for patterns ignores the indexing hint.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Pattern::Verbatim {
                    precedence: _i0,
                    fragment: p1,
                },
                Pattern::Verbatim {
                    precedence: _i1,
                    fragment: p2,
                },
            ) => p1.eq(p2),
            (Pattern::Verbatim { .. }, Pattern::Directory { .. }) => false,
            (Pattern::Directory { .. }, Pattern::Verbatim { .. }) => false,
            (
                Pattern::Directory {
                    precedence: _i0,
                    path: p0,
                    recursive: _r0,
                },
                Pattern::Directory {
                    precedence: _i1,
                    path: p1,
                    recursive: _r1,
                },
            ) => p0.eq(p1),
        }
    }
}

lazy_static! {
    static ref NOT_TOKEN: Vec<u8> = b"!".to_vec();
    static ref STAR_SLASH_TOKEN: Vec<u8> = b"*/".to_vec();
    static ref STAR_TOKEN: Vec<u8> = b"*".to_vec();
    static ref ROOT_PATH: PathBuf = PathBuf::from(String::from(MAIN_SEPARATOR));
}

impl From<Pattern> for Vec<OsString> {
    fn from(other: Pattern) -> Vec<OsString> {
        match other {
            Pattern::Verbatim {
                precedence: _i,
                fragment,
            } => vec![OsString::from(fragment)],
            Pattern::Directory {
                precedence: _i,
                path,
                recursive,
            } => {
                let mut actual = path.as_os_str().as_bytes().to_vec();
                if actual.is_empty() || actual.len() == 1 && actual[0] == MAIN_SEPARATOR_BYTE {
                    return vec![
                        OsString::from_str("/*").unwrap(),
                        OsString::from_str("!/*/").unwrap(),
                    ];
                }

                if !actual.starts_with(MAIN_SEPARATOR_BYTES) {
                    actual.insert(0, MAIN_SEPARATOR_BYTE);
                }
                if !actual.ends_with(MAIN_SEPARATOR_BYTES) {
                    actual.push(MAIN_SEPARATOR_BYTE);
                }

                let no_descendents = {
                    let mut t = NOT_TOKEN.clone();
                    t.extend(actual.clone());
                    t.extend(STAR_SLASH_TOKEN.clone());
                    t
                };

                if recursive {
                    vec![OsString::from_vec(actual)]
                } else {
                    vec![
                        OsString::from_vec(actual),
                        OsString::from_vec(no_descendents),
                    ]
                }
            }
        }
    }
}

/// A set of patterns
pub type PatternSet = BTreeSet<Pattern>;

// A container for patterns, to be loaded as part of repository configuration
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PatternContainer {
    pub patterns: PatternSet,
}

pub trait PatternSetWriter {
    /// Write the Patterns from a PatternSet to a file indicated by the given path, returning a hash digest of the written content.
    fn write_to_file(&self, path: &Path) -> Result<Vec<u8>>;
}

pub trait LeadingPatternInserter {
    fn insert_leading(&mut self, pattern: Pattern);
}

impl PatternSetWriter for PatternSet {
    fn write_to_file(&self, path: &Path) -> Result<Vec<u8>> {
        static ENDLINE: &[u8] = b"\n";

        let mut written_productions = HashSet::<OsString>::new();

        let mut file = File::options()
            .create(true)
            .write(true)
            .open(path)
            .with_context(|| format!("failed opening '{}' for write", path.display()))?;

        let mut digest = Sha256::new();
        for pattern in self.iter() {
            let lines: Vec<OsString> = pattern.clone().into();
            for line in lines {
                if line.as_bytes().eq(MAIN_SEPARATOR_BYTES) {
                    // Skip root patterns (lines that are just "/")
                    continue;
                }
                if !written_productions.insert(line.clone()) {
                    // Skip previously written pattern
                    continue;
                }

                let line_bytes = line.as_bytes();
                digest.update(line_bytes);
                file.write_all(line_bytes)
                    .context("Writing pattern failed")?;
                file.write_all(ENDLINE).context("Writing endline failed")?;
            }
        }

        Ok(digest.finalize().to_vec())
    }
}

impl LeadingPatternInserter for PatternSet {
    fn insert_leading(&mut self, pattern: Pattern) {
        {
            match pattern {
                verbatim @ Pattern::Verbatim { .. } => {
                    self.insert(verbatim);
                }
                Pattern::Directory {
                    precedence,
                    path,
                    recursive: _,
                } => {
                    self.insert(Pattern::Directory {
                        precedence,
                        path: path.clone(),
                        recursive: true,
                    });
                    let current = RefCell::new(path.as_path());
                    loop {
                        let inner = current.clone().into_inner();
                        if let Some(parent) = inner.parent() {
                            // Skip the root path.
                            if parent == ROOT_PATH.as_path() {
                                break;
                            }

                            current.replace(parent);
                            self.insert(Pattern::Directory {
                                precedence,
                                path: parent.to_owned(),
                                recursive: false,
                            });
                        } else {
                            break;
                        }
                    }
                }
            }
        }
    }
}

lazy_static! {
    pub static ref BASELINE_PATTERNS: PatternSet = {
        let mut patterns = PatternSet::new();
        patterns.insert(Pattern::Directory {
            precedence: patterns.len(),
            path: PathBuf::default(),
            recursive: true,
        });
        patterns.insert(Pattern::Directory {
            precedence: usize::MAX,
            path: PathBuf::from("focus"),
            recursive: true,
        });
        patterns
    };
}

#[cfg(test)]
mod testing {
    use std::{ffi::OsString, path::PathBuf};

    use super::*;

    #[test]
    fn verbatim_pattern() {
        let actual: Vec<OsString> = Pattern::Verbatim {
            precedence: 0,
            fragment: String::from("/foo/bar/baz!"),
        }
        .into();
        assert_eq!(actual, vec![OsString::from("/foo/bar/baz!")]);
    }

    #[test]
    fn recursive_directory_pattern() {
        let actual: Vec<OsString> = Pattern::Directory {
            precedence: 0,
            path: PathBuf::from("bar/baz/qux"),
            recursive: true,
        }
        .into();
        assert_eq!(actual, vec![OsString::from("/bar/baz/qux/"),]);
    }

    #[test]
    fn root_recursive_directory_pattern() {
        let actual: Vec<OsString> = Pattern::Directory {
            precedence: 0,
            path: PathBuf::from("/"),
            recursive: true,
        }
        .into();
        assert_eq!(actual, vec![OsString::from("/*"), OsString::from("!/*/")]);
    }

    #[test]
    fn pattern_set_ops() {
        let mut pattern_set = PatternSet::new();
        pattern_set.extend(BASELINE_PATTERNS.clone());
        pattern_set.insert(Pattern::Directory {
            precedence: pattern_set.len(),
            path: PathBuf::from("project_a"),
            recursive: true,
        });
        let count = pattern_set.len();
        pattern_set.retain(|pattern| !BASELINE_PATTERNS.contains(pattern));
        assert_eq!(pattern_set.len(), count - BASELINE_PATTERNS.len());
    }

    #[test]
    fn pattern_partial_ord() {
        let directory_pattern_1 = Pattern::Directory {
            precedence: 0,
            path: PathBuf::from("/"),
            recursive: true,
        };
        let directory_pattern_2 = Pattern::Directory {
            precedence: 0,
            path: PathBuf::from("/b/"),
            recursive: true,
        };
        let verbatim_pattern_1 = Pattern::Verbatim {
            precedence: 1,
            fragment: String::from("bbb"),
        };
        let verbatim_pattern_2 = Pattern::Verbatim {
            precedence: 2,
            fragment: String::from("aaa"),
        };

        let mut pattern_set = PatternSet::new();
        pattern_set.insert(directory_pattern_1.clone());
        pattern_set.insert(directory_pattern_2.clone());
        pattern_set.insert(verbatim_pattern_1.clone());
        pattern_set.insert(verbatim_pattern_2.clone());

        let mut iter = pattern_set.iter();
        // RecursiveDirectory instances are collated by path since the index is the same
        assert_eq!(iter.next().unwrap(), &directory_pattern_1);
        assert_eq!(iter.next().unwrap(), &directory_pattern_2);
        // Verbatim instances follow Directory and are ordered by index
        assert_eq!(iter.next().unwrap(), &verbatim_pattern_1);
        assert_eq!(iter.next().unwrap(), &verbatim_pattern_2);

        assert_eq!(iter.next(), None);
    }

    #[test]
    fn test_insert_leading() {
        let mut pattern_set = PatternSet::new();
        let nested_pattern = Pattern::Directory {
            precedence: 0,
            path: PathBuf::from("/a/b/c/d/e/"),
            recursive: true,
        };
        pattern_set.insert_leading(nested_pattern.clone());
        let mut iter = pattern_set.iter();
        assert_eq!(
            iter.next().unwrap(),
            &Pattern::Directory {
                precedence: 0,
                path: PathBuf::from("/a/"),
                recursive: true,
            }
        );
        assert_eq!(
            iter.next().unwrap(),
            &Pattern::Directory {
                precedence: 0,
                path: PathBuf::from("/a/b/"),
                recursive: true,
            }
        );
        assert_eq!(
            iter.next().unwrap(),
            &Pattern::Directory {
                precedence: 0,
                path: PathBuf::from("/a/b/c/"),
                recursive: true,
            }
        );
        assert_eq!(
            iter.next().unwrap(),
            &Pattern::Directory {
                precedence: 0,
                path: PathBuf::from("/a/b/c/d/"),
                recursive: true,
            }
        );
        assert_eq!(iter.next().unwrap(), &nested_pattern);
    }

    #[test]
    fn test_elimination_of_duplicate_productions() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut patterns = PatternSet::new();
        patterns.insert(Pattern::Directory {
            precedence: 0,
            path: PathBuf::default(),
            recursive: true,
        });
        patterns.insert(Pattern::Verbatim {
            precedence: usize::MAX,
            fragment: String::from("/*"),
        });
        patterns.insert(Pattern::Verbatim {
            precedence: usize::MAX,
            fragment: String::from("!/*/"),
        });
        patterns.write_to_file(file.path()).unwrap();
        insta::assert_snapshot!(std::fs::read_to_string(file.path()).unwrap());
    }
}
