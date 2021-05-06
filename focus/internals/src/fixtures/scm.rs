pub mod testing {
    use crate::error::AppError;
    use anyhow::Result;
    use git2::{IndexAddOption, Oid, Repository, RepositoryInitOptions};
    use log::info;

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
                let new_repo = result.repo().expect("Failed to open new alternate repo");
                let primary_repo = primary.repo().expect("Failed to open primary repo");
                info!(
                    "Setting up {:?} as an alternate of {:?}",
                    &new_repo.path(),
                    &primary_repo.path()
                );
            }

            result
        }

        pub fn path(&self) -> PathBuf {
            self.dir.path().to_owned()
        }

        pub fn repo(&self) -> Result<Repository, AppError> {
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
            ref_to_update: &str,
        ) -> Result<Oid, AppError> {
            assert_eq!(ref_to_update.is_empty(), false);

            let repo = self.repo()?;
            let mut index = repo.index()?;
            index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
            let tree_id = index.write_tree()?;
            index.write()?;
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

            // TODO: Revisit this. The parent commit list must be able to be taken as a sliced reference more easily.
            Ok(repo.commit(Some(ref_to_update), &sig, &sig, message, &tree, &[])?)
        }
    }

    #[test]
    fn test_testrepo() -> Result<(), AppError> {
        let temp_repo = TempRepo::new(false, None, None);
        let repo = temp_repo.repo().unwrap();
        // let mut file = temp_repo.path();
        // let filename = PathBuf::from("a.txt;");
        // file.push(&filename);
        // fs::write(&file, "Hello\n")?;
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
        // repo.commit_signed("A simple commit", signature, None);
        let parents: &[&git2::Commit] = &[]; // First commit.

        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "Simple commit",
            &tree,
            parents,
        )
        .unwrap();

        Ok(())
    }

    #[test]
    fn test_commit_everything() -> Result<(), AppError> {
        let temp_repo = TempRepo::new(false, None, None);
        let repo = temp_repo.repo()?;
        {
            let filename = PathBuf::from("foo.txt");
            temp_repo.create_file(&filename, b"hi\n")?;
            assert_eq!(repo.status_file(&filename)?, git2::Status::WT_NEW);
            temp_repo.commit_everything("blah", "HEAD")?;
            assert_eq!(repo.status_file(&filename)?, git2::Status::CURRENT);
        }
        {
            let filename = PathBuf::from("bar.txt");
            temp_repo.create_file(&filename, b"hi again\n")?;
            assert_eq!(repo.status_file(&filename)?, git2::Status::WT_NEW);
            temp_repo.commit_everything("blah", "HEAD")?;
            assert_eq!(repo.status_file(&filename)?, git2::Status::CURRENT);
        }

        Ok(())
    }
}
