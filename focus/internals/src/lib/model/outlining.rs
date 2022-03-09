use anyhow::{bail, Context, Result};

use std::{
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
    RecursiveDirectory {
        precedence: usize,
        path: std::path::PathBuf,
    },
}

impl PartialOrd for Pattern {
    /// Verbatim patterns always precede RecursiveDirectory patterns. Either are kept in order.
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
            (Pattern::Verbatim { .. }, Pattern::RecursiveDirectory { .. }) => {
                Some(Ordering::Less)
            }
            (Pattern::RecursiveDirectory { .. }, Pattern::Verbatim { .. }) => Some(Ordering::Greater),
            (
                Pattern::RecursiveDirectory {
                    precedence: i0,
                    path: p0,
                },
                Pattern::RecursiveDirectory {
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
            (Pattern::Verbatim { .. }, Pattern::RecursiveDirectory { .. }) => false,
            (Pattern::RecursiveDirectory { .. }, Pattern::Verbatim { .. }) => false,
            (
                Pattern::RecursiveDirectory {
                    precedence: _i0,
                    path: p1,
                },
                Pattern::RecursiveDirectory {
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
            Pattern::RecursiveDirectory {
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

impl From<Pattern> for Vec<OsString> {
    fn from(other: Pattern) -> Vec<OsString> {
        match other {
            Pattern::Verbatim {
                precedence: _i,
                fragment,
            } => vec![OsString::from(fragment)],
            Pattern::RecursiveDirectory {
                precedence: _i,
                path,
            } => {
                let mut forward = path.as_os_str().as_bytes().to_vec();
                if !forward.starts_with(MAIN_SEPARATOR_BYTES) {
                    forward.insert(0, MAIN_SEPARATOR_BYTE);
                }
                if !forward.ends_with(MAIN_SEPARATOR_BYTES) {
                    forward.push(MAIN_SEPARATOR_BYTE);
                }

                vec![OsString::from_vec(forward)]
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

pub trait PatternSetFilter {
    /// Filter out ignored patterns.
    fn retain_relevant(&mut self);
}

impl PatternSetReader for PatternSet {
    fn merge_from_file(&mut self, path: &Path) -> Result<usize> {
        let mut inserted_count: usize = 0;
        let file = File::open(path)
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
            if IGNORED_PATTERNS.contains(pattern) {
                continue;
            }

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

impl PatternSetFilter for PatternSet {
    fn retain_relevant(&mut self) {
        self.retain(|p| !IGNORED_PATTERNS.contains(p))
    }
}

lazy_static! {
    pub static ref BUILD_FILE_PATTERNS: PatternSet = {
        let mut patterns = PatternSet::new();
        patterns.insert(Pattern::Verbatim{precedence:usize::MAX, fragment:String::from("WORKSPACE*")});
        patterns.insert(Pattern::Verbatim{precedence:usize::MAX, fragment:String::from("BUILD*")});
        patterns.insert(Pattern::Verbatim{precedence:usize::MAX, fragment:String::from("*.bzl")});
        patterns
    };
    // TODO(wilhelm): Move these into the mandatory layer where possible with `directory:` entries.
    pub static ref SOURCE_BASELINE_PATTERNS: PatternSet = {
        let mut patterns = PatternSet::new();
        patterns.insert(Pattern::RecursiveDirectory{precedence: 0, path: PathBuf::from("focus")});
        patterns.insert(Pattern::RecursiveDirectory{precedence: 0, path: PathBuf::from("3rdparty")});
        patterns.insert(Pattern::RecursiveDirectory{precedence: 0, path: PathBuf::from("tools")});
        patterns.insert(Pattern::RecursiveDirectory{precedence: 0, path: PathBuf::from("pants-internal")});
        patterns.insert(Pattern::RecursiveDirectory{precedence: 0, path: PathBuf::from("pants-support")});
        patterns
    };
    pub static ref IGNORED_PATTERNS: PatternSet = {
        let mut patterns = PatternSet::new();
        patterns.insert(Pattern::Verbatim{precedence: 0, fragment: String::from("/*")});
        patterns.insert(Pattern::Verbatim{precedence: 1, fragment: String::from("!/*/")});
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
            Pattern::RecursiveDirectory {
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
        let actual: Vec<OsString> = Pattern::RecursiveDirectory {
            precedence: 0,
            path: PathBuf::from("bar/baz/qux"),
        }
        .into();
        assert_eq!(actual, vec![OsString::from("/bar/baz/qux/")]);
    }

    #[test]
    fn recursive_directory_pattern_root_pattern() {
        let actual: Vec<OsString> = Pattern::RecursiveDirectory {
            precedence: 0,
            path: PathBuf::from("/"),
        }
        .into();
        assert_eq!(actual, vec![OsString::from("/")]);
    }

    #[test]
    fn pattern_set_ops() {
        let mut pattern_set = PatternSet::new();
        pattern_set.extend(BUILD_FILE_PATTERNS.clone());
        pattern_set.insert(Pattern::RecursiveDirectory {
            precedence: pattern_set.len(),
            path: PathBuf::from("project_a"),
        });
        let count = pattern_set.len();
        pattern_set.retain(|pattern| !BUILD_FILE_PATTERNS.contains(pattern));
        assert_eq!(pattern_set.len(), count - BUILD_FILE_PATTERNS.len());
    }

    #[test]
    fn retain_relevant() {
        let mut pattern_set = PatternSet::new();
        let top_level = Pattern::Verbatim {
            precedence: 0,
            fragment: String::from("/*"),
        };
        pattern_set.insert(top_level.clone());
        let not_top_level_descendents = Pattern::Verbatim {
            precedence: 1,
            fragment: String::from("!/*/"),
        };
        pattern_set.insert(not_top_level_descendents.clone());
        pattern_set.retain_relevant();
        assert!(!pattern_set.contains(&top_level));
        assert!(!pattern_set.contains(&not_top_level_descendents));
    }

    #[test]
    fn pattern_partial_ord() {
        let recursive_directory_pattern_1 = Pattern::RecursiveDirectory {
            precedence: 0,
            path: PathBuf::from("/b/"),
        };
        let recursive_directory_pattern_2 = Pattern::RecursiveDirectory {
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
}
