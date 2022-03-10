use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;

use sha1::digest::DynDigest;
use sha1::Digest;
use sha1::Sha1;

use super::DependencyKey;

type Hasher<'a> = &'a mut dyn DynDigest;

/// The hash of a [`DependencyKey`]'s syntactic content.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash(git2::Oid);

impl Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self(hash) = self;
        write!(f, "{}", hash)
    }
}

impl FromStr for ContentHash {
    type Err = git2::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let oid = git2::Oid::from_str(s)?;
        Ok(ContentHash(oid))
    }
}

/// Indicates that the implementing type can be content-hashed with respect to a
/// state of the repository. Callers will want to use
/// [`ContentHashable::content_hash`].
pub trait ContentHashable {
    /// Write values into the provided `hasher` according to this item's
    /// content-addressable state.
    ///
    /// In order to hash the [`ContentHashable`] values which make up the
    /// current item, the implementor can call [`ContentHashable::write`] on
    /// those values recursively.
    fn write(&self, hasher: Hasher, head_tree: &git2::Tree) -> anyhow::Result<()>;

    /// Construct a hasher, hash this value, finalize the hash, and return the
    /// overall hash of this value.
    fn content_hash(&self, head_tree: &git2::Tree) -> anyhow::Result<ContentHash> {
        let mut hasher = Sha1::new();
        self.write(&mut hasher, head_tree)?;
        let result = hasher.finalize();
        let oid = git2::Oid::from_bytes(&result)?;
        Ok(ContentHash(oid))
    }
}

impl ContentHashable for PathBuf {
    fn write(&self, hasher: Hasher, head_tree: &git2::Tree) -> anyhow::Result<()> {
        hasher.update(b"PathBuf(");

        match head_tree.get_path(self) {
            Ok(entry) => {
                hasher.update(entry.id().as_bytes());
            }
            Err(err) if err.code() == git2::ErrorCode::NotFound => {
                // TODO: test this code path
                hasher.update(git2::Oid::zero().as_bytes());
            }
            Err(err) => return Err(err.into()),
        };

        hasher.update(b")");
        Ok(())
    }
}

impl ContentHashable for DependencyKey {
    fn write(&self, hasher: Hasher, head_tree: &git2::Tree) -> anyhow::Result<()> {
        hasher.update(b"DependencyKey");

        match self {
            DependencyKey::BazelPackage {
                external_repository: None,
                path,
            } => {
                hasher.update(b"::BazelPackage(");
                path.write(hasher, head_tree)?;
            }

            DependencyKey::Path(path) => {
                hasher.update(b"::Path(");
                path.write(hasher, head_tree)?;
            }

            DependencyKey::BazelPackage {
                external_repository: Some(_external_package),
                path: _,
            } => {
                todo!("establish dependency for path in external package")
            }

            DependencyKey::BazelBuildFile(_) => {
                // TODO: hash `BUILD` file contents
                // TODO: parse `load` dependencies out of the `BUILD` file and mix
                // into hash.
                todo!("hash bazel file dep")
            }
        };

        hasher.update(b")");
        Ok(())
    }
}
