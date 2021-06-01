pub mod testing {
    use crate::error::AppError;
    use anyhow::Result;
    use git2::{IndexAddOption, Oid, Repository, RepositoryInitOptions};
    use log::{debug, info, warn};

    use env_logger::Env;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    pub struct TempRepo {
        dir: TempDir,
        opts: RepositoryInitOptions,
        bare: bool,
    }

    impl TempRepo {
        pub fn new(
            bare: bool,
            alternate_of: Option<&TempRepo>,
            origin_url: Option<&str>,
        ) -> TempRepo {
            let dir = TempDir::new().expect("Could not create temporary directory");

            let mut opts = RepositoryInitOptions::new();
            opts.bare(bare);
            opts.description("Test repository");
            opts.workdir_path(&dir.path());

            if let Some(url) = origin_url {
                opts.origin_url(url);
            }

            let result = TempRepo { dir, opts, bare };

            if let Some(primary) = alternate_of {
                let new_repo = result
                    .underlying()
                    .expect("Failed to open new alternate repo");
                let primary_repo = primary.underlying().expect("Failed to open primary repo");
                info!(
                    "Setting up {:?} as an alternate of {:?}",
                    &new_repo.path(),
                    &primary_repo.path()
                );
            }

            result
        }

        pub fn new_with_stuff() -> Result<(Self, Oid), AppError> {
            let temp_repo = TempRepo::new(false, None, None);
            let underlying = temp_repo.underlying()?;
            let mut oid: Oid;
            let repo = temp_repo.underlying()?;
            {
                let filename = PathBuf::from("a/foo.txt");
                temp_repo.create_file(&filename, b"hi\n")?;
                assert_eq!(repo.status_file(&filename)?, git2::Status::WT_NEW);
                temp_repo.commit_everything("blah", &[&filename], "refs/heads/main")?;
                assert_eq!(repo.status_file(&filename)?, git2::Status::CURRENT);
            }
            {
                let filename = PathBuf::from("b/bar.txt");
                temp_repo.create_file(&filename, b"hi again\n")?;
                assert_eq!(repo.status_file(&filename)?, git2::Status::WT_NEW);
                oid = temp_repo.commit_everything("blah", &[&filename], "refs/heads/main")?;
                assert_eq!(repo.status_file(&filename)?, git2::Status::CURRENT);
            }
            Ok((temp_repo, oid))
        }

        pub fn path(&self) -> PathBuf {
            self.dir.path().to_owned()
        }

        pub fn underlying(&self) -> Result<Repository, AppError> {
            Ok(Repository::init_opts(self.dir.path(), &self.opts)?)
        }

        pub fn bare(&self) -> bool {
            self.bare
        }

        pub fn create_file(&self, path: &Path, content: &[u8]) -> Result<(), AppError> {
            use std::fs::File;
            use std::io::prelude::*;

            let final_path = self.path().join(path);

            {
                // Ensure the directory the file is being placed in exists.
                let mut dir = final_path.clone();
                dir.pop();
                std::fs::create_dir_all(dir)
                    .expect("Failed ensuring directory for the new file exists");
            }

            {
                // Create the file and write its contents
                let mut f = File::create(final_path)?;
                f.write_all(&content)?;
            }

            Ok(())
        }

        pub fn commit_everything(
            &self,
            message: &str,
            paths: &[&PathBuf],
            ref_to_update: &str,
        ) -> Result<Oid, AppError> {
            assert_eq!(ref_to_update.is_empty(), false);

            let repo = self.underlying()?;
            let mut index = repo.index()?;

            let prev_dir = std::env::current_dir()?;
            if let Some(work_dir) = repo.workdir() {
                std::env::set_current_dir(work_dir);
            } else {
                return Err(AppError::InvalidWorkDir())
            }
            for &path in paths {
                debug!("Adding {:?}", &path);
                index.add_path(&path);
            }
            std::env::set_current_dir(prev_dir)?;
            assert_eq!(index.len(), paths.len());
            index.write()?;
            let tree_id = index.write_tree()?;
            let tree = repo.find_tree(tree_id)?;
            let sig = repo.signature()?;

            if let Ok(id) = repo.refname_to_id(ref_to_update) {
                if let Ok(parent_commit) = repo.find_commit(id) {
                    return Ok(repo.commit(
                        Some(ref_to_update),
                        &sig,
                        &sig,
                        message,
                        &tree,
                        &[&parent_commit],
                    )?);
                }
            }

            let commit_oid = repo.commit(None, &sig, &sig, message, &tree, &[])?;
            repo.reference(ref_to_update, commit_oid, false, "Created ref");

            // TODO: Revisit this. The parent commit list must be able to be taken as a sliced reference more easily.
            Ok(commit_oid)
        }
    }

    #[test]
    fn test_testrepo() -> Result<(), AppError> {
        let temp_repo = TempRepo::new(false, None, None);
        let repo = temp_repo.underlying().unwrap();
        let filename = PathBuf::from("a.txt");
        temp_repo.create_file(&filename, b"Why, hello!\n")?;
        assert_eq!(repo.status_file(&filename).unwrap(), git2::Status::WT_NEW);
        let mut index = repo.index().expect("Failed to open the index");
        index.add_path(&filename).expect("Failed to add path");
        assert_eq!(repo.status_file(&filename)?, git2::Status::INDEX_NEW);
        assert_eq!(index.has_conflicts(), false);

        let another_filename = PathBuf::from("foo/bar/b.txt");
        temp_repo.create_file(&another_filename, b"Hello, again!\n")?;
        assert_eq!(repo.status_file(&another_filename)?, git2::Status::WT_NEW);
        index
            .add_path(&another_filename)
            .expect("Failed to add path");
        assert_eq!(
            repo.status_file(&another_filename)?,
            git2::Status::INDEX_NEW
        );
        assert_eq!(index.has_conflicts(), false);

        let tree_oid = index.write_tree()?;
        index.write()?;
        let tree = repo.find_tree(tree_oid)?;

        let signature = repo.signature()?;
        let parents: &[&git2::Commit] = &[]; // First commit.

        let commit_oid = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "Simple commit",
                &tree,
                parents,
            )
            .unwrap();

        repo.reference("refs/heads/main", commit_oid, true, "Create ref")?;

        Ok(())
    }

    #[test]
    fn test_commit_everything() -> Result<(), AppError> {
        env_logger::Builder::from_env(Env::default().default_filter_or("debug")).init();
        let temp_repo = TempRepo::new(false, None, None);
        let repo = temp_repo.underlying()?;
        {
            let filename = PathBuf::from("foo.txt");
            temp_repo.create_file(&filename, b"hi\n")?;
            assert_eq!(repo.status_file(&filename)?, git2::Status::WT_NEW);
            temp_repo.commit_everything("blah", &[&filename], "refs/heads/main")?;
            assert_eq!(repo.status_file(&filename)?, git2::Status::CURRENT);
        }
        {
            let filename = PathBuf::from("bar.txt");
            temp_repo.create_file(&filename, b"hi again\n")?;
            assert_eq!(repo.status_file(&filename)?, git2::Status::WT_NEW);
            temp_repo.commit_everything("blah", &[&filename], "refs/heads/main")?;
            assert_eq!(repo.status_file(&filename)?, git2::Status::CURRENT);
        }

        Ok(())
    }
}
