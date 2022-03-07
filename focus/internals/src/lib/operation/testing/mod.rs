#[cfg(test)]
pub(in crate::operation) mod refs {
    use std::{
        path::{Path, PathBuf},
        process::Command,
        sync::Arc,
    };

    use anyhow::{Context, Result};

    use crate::app::App;
    use git2::Repository;

    use tempfile::TempDir;
    use uuid::Uuid;

    pub struct Fixture {
        _tempdir: TempDir,
        repo_path: PathBuf,
        app: Arc<App>,
        repo: Repository,
    }

    impl Fixture {
        pub fn new() -> Result<Fixture> {
            let _tempdir = tempfile::tempdir()?;
            let repo_path = Self::init(_tempdir.path())?;

            let app = Arc::new(App::new(false, false)?);
            let repo = git2::Repository::open(&repo_path).context("failed to open Repository")?;

            Ok(Fixture {
                _tempdir,
                repo_path,
                app,
                repo,
            })
        }

        /// creates the temp repo and returns the path created
        fn init(containing_dir: &Path) -> Result<PathBuf> {
            let name = format!("repo_{}", Uuid::new_v4().to_string());
            Command::new("git")
                .arg("init")
                .arg(&name)
                .current_dir(containing_dir)
                .status()
                .context("git init failed")?;

            let repo_path = containing_dir.join(&name);

            Command::new("git")
                .arg("switch")
                .arg("-c")
                .arg("main")
                .current_dir(&repo_path)
                .status()
                .context("git switch failed")?;

            Ok(repo_path)
        }

        pub fn repo_path(&self) -> &Path {
            &self.repo_path
        }

        pub fn repo(&self) -> &Repository {
            &self.repo
        }

        pub fn app(&self) -> Arc<App> {
            self.app.clone()
        }

        pub fn write<P: AsRef<Path>, B: AsRef<[u8]>>(&self, relpath: P, content: B) -> Result<()> {
            let filename = self.repo_path().join(relpath);
            std::fs::write(filename, content.as_ref()).context("writing content")
        }

        pub fn add<P: AsRef<Path>>(&self, relpath: P) -> Result<()> {
            self.repo()
                .index()?
                .add_path(relpath.as_ref())
                .context("failed to add path to index")
        }

        pub fn commit<S: AsRef<str>>(
            &mut self,
            message: S,
            author_opt: Option<&git2::Signature>,
            committer_opt: Option<&git2::Signature>,
        ) -> Result<git2::Oid> {
            let repo = self.repo();
            let mut index = repo.index()?;
            let tree_oid = index.write_tree()?;
            let tree = repo.find_tree(tree_oid)?;

            let author = match author_opt {
                Some(a) => a.to_owned(),
                None => repo.signature()?.clone(),
            };

            let committer = match committer_opt {
                Some(a) => a.to_owned(),
                None => repo.signature()?.clone(),
            };

            // figure out if HEAD is pointing to a born branch yet
            let parents = match repo.head() {
                Ok(head) => match head.peel_to_commit() {
                    Ok(commit) => vec![commit],
                    Err(_e) => vec![],
                },
                Err(_) => {
                    vec![]
                }
            };

            let pref: Vec<&git2::Commit> = parents.iter().collect();

            repo.commit(
                Some("HEAD"),
                &author,
                &committer,
                message.as_ref(),
                &tree,
                &pref,
            )
            .context("commit failed")
        }

        pub fn create_branch<S: AsRef<str>>(
            &self,
            name: S,
            oid: git2::Oid,
        ) -> Result<git2::Reference> {
            self.repo()
                .reference(name.as_ref(), oid, false, "")
                .with_context(|| format!("failed to create reference {}", name.as_ref()))
        }

        pub fn checkout_b<S: AsRef<str>>(
            &mut self,
            branch_name: S,
            oid: git2::Oid,
        ) -> Result<git2::Reference> {
            let br = self.create_branch(branch_name, oid)?;
            self.repo().set_head(&br.name().unwrap())?;
            self.repo().reset(
                &br.peel_to_commit()?.as_object(),
                git2::ResetType::Soft,
                Some(git2::build::CheckoutBuilder::new().safe()),
            )?;

            Ok(br)
        }

        pub fn checkout<S: AsRef<str>>(
            &mut self,
            branch_name: S,
            reset_type: Option<git2::ResetType>,
        ) -> Result<()> {
            let repo = self.repo();
            repo.set_head(branch_name.as_ref())?;
            repo.reset(
                repo.find_reference(branch_name.as_ref())?
                    .peel_to_commit()?
                    .as_object(),
                reset_type.unwrap_or(git2::ResetType::Mixed),
                Some(git2::build::CheckoutBuilder::new().safe()),
            )
            .with_context(|| format!("failed to checkout {}", branch_name.as_ref()))
        }

        pub fn write_add_and_commit<P: AsRef<Path>, S: AsRef<str>, B: AsRef<[u8]>>(
            &mut self,
            filename: P,
            content: B,
            message: S,
            author_opt: Option<&git2::Signature>,
            committer_opt: Option<&git2::Signature>,
        ) -> Result<git2::Oid> {
            self.write(&filename, content)?;
            self.add(&filename)?;
            self.commit(message, author_opt, committer_opt)
        }
    }
}
