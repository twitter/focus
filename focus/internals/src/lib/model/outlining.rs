use anyhow::{bail, Context, Result};
use tracing::debug;

use std::{
    cell::RefCell,
    cmp::Ordering,
    collections::BTreeSet,
    ffi::OsString,
    fs::File,
    io::{BufRead, BufReader, Write},
    os::unix::prelude::{OsStrExt, OsStringExt},
    path::{Path, PathBuf, MAIN_SEPARATOR},
};

use lazy_static::lazy_static;

const MAIN_SEPARATOR_BYTE: u8 = MAIN_SEPARATOR as u8;
const MAIN_SEPARATOR_BYTES: &[u8] = &[MAIN_SEPARATOR_BYTE];

#[derive(Clone, Debug, Eq)]
pub enum Pattern {
    Verbatim {
        precedence: usize,
        fragment: String,
    },
    Directory {
        precedence: usize,
        path: std::path::PathBuf,
    },
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
            (Pattern::Verbatim { .. }, Pattern::Directory { .. }) => Some(Ordering::Less),
            (Pattern::Directory { .. }, Pattern::Verbatim { .. }) => Some(Ordering::Greater),
            (
                Pattern::Directory {
                    precedence: i0,
                    path: p0,
                },
                Pattern::Directory {
                    precedence: i1,
                    path: p1,
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
                    path: p1,
                },
                Pattern::Directory {
                    precedence: _i1,
                    path: p2,
                },
            ) => p1.eq(p2),
        }
    }
}

impl From<(usize, String)> for Pattern {
    fn from(pair: (usize, String)) -> Self {
        let (index, s) = pair;
        if s.starts_with(MAIN_SEPARATOR) && s.ends_with(MAIN_SEPARATOR) {
            Pattern::Directory {
                precedence: index,
                path: PathBuf::from(s),
            }
        } else {
            Pattern::Verbatim {
                precedence: index,
                fragment: s,
            }
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
            } => {
                let mut actual = path.as_os_str().as_bytes().to_vec();
                if !actual.starts_with(MAIN_SEPARATOR_BYTES) {
                    actual.insert(0, MAIN_SEPARATOR_BYTE);
                }
                if !actual.ends_with(MAIN_SEPARATOR_BYTES) {
                    actual.push(MAIN_SEPARATOR_BYTE);
                }

                let wildcard = {
                    let mut t = actual.clone();
                    t.extend(STAR_TOKEN.clone());
                    t
                };
                let no_descendents = {
                    let mut t = NOT_TOKEN.clone();
                    t.extend(actual.clone());
                    t.extend(STAR_SLASH_TOKEN.clone());
                    t
                };

                vec![
                    OsString::from_vec(wildcard),
                    OsString::from_vec(no_descendents),
                ]
            }
        }
    }
}

/// A sorted for Pattern instances. Backed by a BTreeSet.
pub type PatternSet = BTreeSet<Pattern>;

pub trait PatternSetReader {
    /// Merge the Patterns from a file indicated by the given path into a PatternSet.
    fn merge_from_file(&mut self, path: &Path) -> Result<usize>;
}

pub trait PatternSetWriter {
    /// Write the Patterns from a PatternSet to a file indicated by the given path.
    fn write_to_file(&self, path: &Path) -> Result<()>;
}

pub trait LeadingPatternInserter {
    fn insert_leading(&mut self, pattern: Pattern);
}

impl PatternSetReader for PatternSet {
    fn merge_from_file(&mut self, path: &Path) -> Result<usize> {
        let mut inserted_count: usize = 0;
        let file = File::create(path)
            .with_context(|| format!("failed opening '{}' for read", path.display()))?;
        let buffered_reader = BufReader::new(file).lines();
        for (line_number, line) in buffered_reader.enumerate() {
            match line {
                Ok(content) => {
                    let item = Pattern::from((line_number, content));
                    if self.insert(item) {
                        inserted_count += 1;
                    }
                }
                Err(e) => bail!("buffered read failed: {}", e),
            }
        }

        Ok(inserted_count)
    }
}

impl PatternSetWriter for PatternSet {
    fn write_to_file(&self, path: &Path) -> Result<()> {
        static ENDLINE: &[u8] = b"\n";

        let mut file = File::options()
            .create(true)
            .write(true)
            .open(path)
            .with_context(|| format!("failed opening '{}' for write", path.display()))?;

        for pattern in self.iter() {
            let lines: Vec<OsString> = pattern.clone().into();
            for line in lines {
                if line.as_bytes().eq(MAIN_SEPARATOR_BYTES) {
                    // Skip root patterns (lines that are just "/")
                    continue;
                }

                file.write_all(line.as_bytes())
                    .context("Writing pattern failed")?;
                file.write_all(ENDLINE).context("Writing endline failed")?;
            }
        }

        Ok(())
    }
}

impl LeadingPatternInserter for PatternSet {
    fn insert_leading(&mut self, pattern: Pattern) {
        {
            match pattern {
                verbatim @ Pattern::Verbatim { .. } => {
                    self.insert(verbatim);
                }
                Pattern::Directory { precedence, path } => {
                    debug!(path = %path.display(), "Adding leading patterns");
                    self.insert(Pattern::Directory {
                        precedence,
                        path: path.clone(),
                    });
                    let current = RefCell::new(path.as_path());
                    loop {
                        let inner = current.clone().into_inner();
                        if let Some(parent) = inner.parent() {
                            // Skip the root path.
                            if parent == ROOT_PATH.as_path() {
                                break;
                            }
                            debug!(path = %parent.display(), "Current");

                            current.replace(parent);
                            self.insert(Pattern::Directory {
                                precedence,
                                path: parent.to_owned(),
                            });
                        } else {
                            break;
                        }
                    }
                    debug!("Finished");
                }
            }
        }
    }
}

lazy_static! {
    pub static ref GIT_BASELINE_PATTERNS: PatternSet = {
        let mut patterns = PatternSet::new();
        patterns.insert(Pattern::Verbatim {
            precedence: 0,
            fragment: String::from("/*"),
        });
        patterns.insert(Pattern::Verbatim {
            precedence: 1,
            fragment: String::from("!/*/"),
        });
        patterns
    };
    pub static ref SOURCE_BASELINE_PATTERNS: PatternSet = {
        let mut patterns = PatternSet::new();
        patterns.insert(Pattern::Verbatim {
            precedence: 2,
            fragment: String::from("/3rdparty/*"),
        });
        patterns.insert(Pattern::Verbatim {
            precedence: 3,
            fragment: String::from("/focus/*"),
        });
        patterns.insert(Pattern::Verbatim {
            precedence: 4,
            fragment: String::from("/tools/*"),
        });
        patterns.insert(Pattern::Verbatim {
            precedence: 5,
            fragment: String::from("/pants-internal/*"),
        });
        patterns.insert(Pattern::Verbatim {
            precedence: 6,
            fragment: String::from("/pants-support/*"),
        });
        patterns
    };
    pub static ref BUILD_FILE_PATTERNS: PatternSet = {
        let mut patterns = GIT_BASELINE_PATTERNS.clone();
        patterns.insert(Pattern::Verbatim {
            precedence: usize::MAX,
            fragment: String::from("WORKSPACE*"),
        });
        patterns.insert(Pattern::Verbatim {
            precedence: usize::MAX,
            fragment: String::from("BUILD*"),
        });
        patterns.insert(Pattern::Verbatim {
            precedence: usize::MAX,
            fragment: String::from("*.bzl"),
        });
        patterns
    };
}

#[cfg(test)]
mod testing {
    use std::{ffi::OsString, path::PathBuf};

    use super::*;

    #[test]
    fn from_string() {
        assert_eq!(
            Pattern::Verbatim {
                precedence: 0,
                fragment: String::from("/boo!")
            },
            Pattern::from((0, String::from("/boo!")))
        );
        assert_eq!(
            Pattern::Directory {
                precedence: 0,
                path: PathBuf::from("/a/b/c/")
            },
            Pattern::from((0, String::from("/a/b/c/")))
        );
    }

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
        }
        .into();
        assert_eq!(
            actual,
            vec![
                OsString::from("/bar/baz/qux/*"),
                OsString::from("!/bar/baz/qux/*/")
            ]
        );
    }

    #[test]
    fn recursive_directory_pattern_root_pattern() {
        let actual: Vec<OsString> = Pattern::Directory {
            precedence: 0,
            path: PathBuf::from("/"),
        }
        .into();
        assert_eq!(actual, vec![OsString::from("/*"), OsString::from("!/*/")]);
    }

    #[test]
    fn pattern_set_ops() {
        let mut pattern_set = PatternSet::new();
        pattern_set.extend(BUILD_FILE_PATTERNS.clone());
        pattern_set.insert(Pattern::Directory {
            precedence: pattern_set.len(),
            path: PathBuf::from("project_a"),
        });
        let count = pattern_set.len();
        pattern_set.retain(|pattern| !BUILD_FILE_PATTERNS.contains(pattern));
        assert_eq!(pattern_set.len(), count - BUILD_FILE_PATTERNS.len());
    }

    #[test]
    fn pattern_partial_ord() {
        let recursive_directory_pattern_1 = Pattern::Directory {
            precedence: 0,
            path: PathBuf::from("/b/"),
        };
        let recursive_directory_pattern_2 = Pattern::Directory {
            precedence: 0,
            path: PathBuf::from("/a/"),
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
        pattern_set.insert(recursive_directory_pattern_1.clone());
        pattern_set.insert(recursive_directory_pattern_2.clone());
        pattern_set.insert(verbatim_pattern_1.clone());
        pattern_set.insert(verbatim_pattern_2.clone());

        let mut iter = pattern_set.iter();
        // Verbatim instances precede RecrsiveDirectory and are ordered by index
        assert_eq!(iter.next().unwrap(), &verbatim_pattern_1);
        assert_eq!(iter.next().unwrap(), &verbatim_pattern_2);
        // RecursiveDirectory instances are collated by path since the index is the same
        assert_eq!(iter.next().unwrap(), &recursive_directory_pattern_2);
        assert_eq!(iter.next().unwrap(), &recursive_directory_pattern_1);

        assert_eq!(iter.next(), None);
    }

    #[test]
    fn test_insert_leading() {
        let mut pattern_set = PatternSet::new();
        let nested_pattern = Pattern::Directory {
            precedence: 0,
            path: PathBuf::from("/a/b/c/d/e/"),
        };
        pattern_set.insert_leading(nested_pattern.clone());
        let mut iter = pattern_set.iter();
        assert_eq!(
            iter.next().unwrap(),
            &Pattern::Directory {
                precedence: 0,
                path: PathBuf::from("/a/")
            }
        );
        assert_eq!(
            iter.next().unwrap(),
            &Pattern::Directory {
                precedence: 0,
                path: PathBuf::from("/a/b/")
            }
        );
        assert_eq!(
            iter.next().unwrap(),
            &Pattern::Directory {
                precedence: 0,
                path: PathBuf::from("/a/b/c/")
            }
        );
        assert_eq!(
            iter.next().unwrap(),
            &Pattern::Directory {
                precedence: 0,
                path: PathBuf::from("/a/b/c/d/")
            }
        );
        assert_eq!(iter.next().unwrap(), &nested_pattern);
    }
}
