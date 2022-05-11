pub mod production;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    fmt::Display,
    fs::File,
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
};
use tracing::{debug, info};

/// Migration instance should implement this trait.
pub trait Migration {
    fn id(&self) -> Identifier;
    fn description(&self) -> &str;
    fn upgrade(&self, path: &Path) -> Result<()>;
}

pub type Migrations = Vec<Box<dyn Migration>>;

/// Identifies a migration
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Identifier {
    Serial(u64),
}

impl Display for Identifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Identifier::Serial(serial) => write!(f, "#{}", serial),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Manifest {
    version: Cell<Identifier>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: Cell::new(Identifier::Serial(0)),
        }
    }
}

pub struct Runner {
    repo_path: PathBuf,
    manifest_path: PathBuf,
    manifest: RefCell<Manifest>,
    migrations: Migrations,
}

impl Runner {
    pub fn new(repo_path: &Path, manifest_path: &Path, migrations: Migrations) -> Result<Self> {
        let instance = Self {
            repo_path: repo_path.to_owned(),
            manifest_path: manifest_path.to_owned(),
            manifest: Default::default(),
            migrations,
        };
        if let Err(error) = instance.load_manifest() {
            debug!(%error, "Failed to load manifest");
        }
        Ok(instance)
    }

    fn load_manifest(&self) -> Result<()> {
        let reader = BufReader::new(File::open(&self.manifest_path).with_context(|| {
            format!(
                "Failed to open manifest from {}",
                self.manifest_path.display()
            )
        })?);
        self.manifest
            .replace(serde_json::from_reader(reader).context("Failed to read manifest")?);
        Ok(())
    }

    fn store_manifest(&self) -> Result<()> {
        let writer =
            BufWriter::new(File::create(&self.manifest_path).with_context(|| {
                format!("Opening manifest at {}", self.manifest_path.display())
            })?);
        serde_json::to_writer(writer, &self.manifest)
            .context("Failed writing serialized content")?;
        Ok(())
    }

    fn ultimate_migration(&self) -> Option<Identifier> {
        self.migrations.last().map(|m| m.as_ref().id())
    }

    pub fn is_upgrade_required(&self) -> Result<bool> {
        if let Some(ultimate_version) = self.ultimate_migration() {
            return Ok(self.manifest.borrow().version.get() < ultimate_version);
        }

        Ok(false)
    }

    pub fn perform_pending_migrations(&self) -> Result<bool> {
        // Iterate through migrations. Keep track of the highest one that succeeded. Make sure to mark those that have been performed as we go. If one fails, stop.
        let previous_version = &self.manifest.borrow().version;
        for migration in self
            .migrations
            .iter()
            .skip_while(|&m| m.as_ref().id() < previous_version.get())
        {
            let migration = migration.as_ref();
            let identifier = migration.id();
            let description = migration.description();
            info!(%identifier, %description, "Running migration");
            match migration.upgrade(&self.repo_path) {
                Ok(()) => {
                    self.manifest.borrow().version.replace(identifier);
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }

        self.store_manifest()
            .context("Failed to store the manifest")?;

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{bail, Result};
    use std::path::PathBuf;
    use tempfile::TempDir;

    use super::*;

    #[allow(dead_code)]
    struct Fixture {
        pub(crate) dir: TempDir,
        pub(crate) repo_dir: PathBuf,
        pub(crate) manifest_path: PathBuf,
    }

    impl Fixture {
        fn new() -> Result<Self> {
            let dir = tempfile::tempdir()?;
            let repo_dir = dir.path().join("repo");
            std::fs::create_dir(&repo_dir)?;
            let manifest_path = dir.path().join("manifest.json");

            Ok(Self {
                dir,
                repo_dir,
                manifest_path,
            })
        }

        fn new_runner_with_migrations(&self, migrations: Migrations) -> Result<Runner> {
            Runner::new(&self.repo_dir, &self.manifest_path, migrations)
        }
    }

    struct SuccessfulMigration;
    impl Migration for SuccessfulMigration {
        fn id(&self) -> Identifier {
            Identifier::Serial(1)
        }

        fn description(&self) -> &str {
            "A migration that succeeds for use in tests"
        }

        fn upgrade(&self, _path: &Path) -> Result<()> {
            Ok(())
        }
    }

    struct FailingMigration;
    impl Migration for FailingMigration {
        fn id(&self) -> Identifier {
            Identifier::Serial(2)
        }

        fn description(&self) -> &str {
            "A migration that fails for use in tests"
        }

        fn upgrade(&self, _path: &Path) -> Result<()> {
            bail!("boom")
        }
    }

    #[test]
    fn test_no_migrations() -> Result<()> {
        let fixture = Fixture::new()?;
        let runner = fixture.new_runner_with_migrations(vec![])?;
        assert!(!runner.is_upgrade_required()?);
        Ok(())
    }

    #[test]
    fn test_is_upgrade_required() -> Result<()> {
        let migration = SuccessfulMigration {};
        let migrations: Vec<Box<dyn Migration>> = vec![Box::new(migration)];
        let fixture = Fixture::new()?;
        let runner = fixture.new_runner_with_migrations(migrations)?;
        assert!(runner.is_upgrade_required()?);
        Ok(())
    }

    #[test]
    fn perform_pending_migrations_with_a_failed_migration_does_not_update_version() -> Result<()> {
        let failing_migration = FailingMigration {};
        let migrations: Vec<Box<dyn Migration>> = vec![Box::new(failing_migration)];
        let fixture = Fixture::new()?;
        let runner = fixture.new_runner_with_migrations(migrations)?;
        assert!(runner.is_upgrade_required()?);
        let error = runner.perform_pending_migrations().unwrap_err();
        assert_eq!(error.to_string(), "boom");

        assert!(runner.is_upgrade_required()?);
        Ok(())
    }

    #[test]
    fn perform_pending_migrations_with_a_successful_migration_updates_the_version() -> Result<()> {
        let successful_migration = SuccessfulMigration {};
        let migrations: Vec<Box<dyn Migration>> = vec![Box::new(successful_migration)];
        let fixture = Fixture::new()?;
        let runner = fixture.new_runner_with_migrations(migrations)?;
        assert!(runner.is_upgrade_required()?);
        assert!(runner.perform_pending_migrations()?);
        assert!(!runner.is_upgrade_required()?);

        Ok(())
    }

    #[test]
    fn manifest_persists_after_upgrade() -> Result<()> {
        let fixture = Fixture::new()?;

        {
            let runner =
                fixture.new_runner_with_migrations(vec![Box::new(SuccessfulMigration {})])?;
            assert!(runner.perform_pending_migrations()?);
            assert!(!runner.is_upgrade_required()?);
        }

        {
            let runner =
                fixture.new_runner_with_migrations(vec![Box::new(SuccessfulMigration {})])?;
            assert!(!runner.is_upgrade_required()?);
        }

        Ok(())
    }
}
