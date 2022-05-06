use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ffi::OsString,
    fmt::Display,
    os::unix::prelude::OsStrExt,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Error, Result};

use serde_derive::{Deserialize, Serialize};
use tracing::{debug, warn};
use walkdir::{DirEntry, WalkDir};

#[derive(thiserror::Error, Debug)]
pub enum RemovalError {
    #[error("not found")]
    NotFound,

    #[error("unable to remove mandatory project")]
    Mandatory,
}

#[derive(thiserror::Error, Debug)]
pub enum LoadError {
    #[error("not found")]
    NotFound,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, PartialOrd)]
pub struct Project {
    name: String,

    description: String,

    #[serde(default)]
    mandatory: bool,

    targets: Vec<String>,
}

impl Display for Project {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}{} ({}) -> {:?}",
            &self.name,
            if self.mandatory { " <mandatory>" } else { "" },
            &self.description,
            &self.targets,
        )
    }
}

impl Project {
    pub fn new(name: &str, description: &str, mandatory: bool, targets: Vec<String>) -> Self {
        Self {
            name: name.to_owned(),
            description: description.to_owned(),
            mandatory,
            targets,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn targets(&self) -> &[String] {
        &self.targets
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, PartialOrd)]
pub struct ProjectSet {
    projects: Vec<Project>,
}

impl ProjectSet {
    pub fn new(projects: Vec<Project>) -> Self {
        Self { projects }
    }

    pub fn validate(&self) -> Result<()> {
        // Find duplicate names
        let mut visited_names = HashMap::<String, usize>::new();
        for (index, project) in self.projects.iter().enumerate() {
            let name = &project.name.to_owned();
            if name.contains(':') {
                bail!(
                    "Project name '{}' contains a colon (:); colons are not allowed in project names",
                    name
                );
            } else if let Some(existing_index) = visited_names.insert(name.to_owned(), index) {
                bail!(
                    "Project named '{}' at index {} has the same name as existing project at index {}",
                    &name,
                    index,
                    existing_index
                );
            }
        }
        Ok(())
    }

    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    pub fn extend(&mut self, other: ProjectSet) {
        self.projects.extend(other.projects);
    }

    pub fn remove_named_project(&mut self, name: &str) -> Result<()> {
        for (ix, l) in self.projects.iter().enumerate() {
            if l.name.eq(&name) {
                if l.mandatory {
                    return Err(Error::new(RemovalError::Mandatory));
                }
                self.projects.remove(ix);
                return Ok(());
            }
        }

        Err(Error::new(RemovalError::NotFound))
    }

    pub fn optional_projects(&self) -> Result<Vec<&Project>> {
        Ok(self.projects.iter().filter(|l| !l.mandatory).collect())
    }

    fn load(path: &Path) -> Result<ProjectSet> {
        let slice =
            &std::fs::read(&path).with_context(|| format!("opening file {:?} for read", &path))?;

        let project_set: ProjectSet = serde_json::from_slice(slice)
            .with_context(|| format!("loading project set from {}", &path.display()))?;
        Ok(project_set)
    }

    fn store(path: &Path, t: &ProjectSet) -> Result<()> {
        let parent = path.parent().context("determining parent directory")?;
        if !parent.is_dir() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "creating the directory ({}) to to store the project set in",
                    parent.display()
                )
            })?;
        }

        std::fs::write(
            &path,
            &serde_json::to_vec_pretty(&t).context("opening file for write")?,
        )
        .context("storing project_set")?;

        Ok(())
    }
}

// Selections are stacks of names of projects.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, PartialOrd)]
pub struct ProjectStack {
    selected_project_names: Vec<String>,
}

impl ProjectStack {
    pub fn load(path: &Path) -> Result<ProjectStack> {
        serde_json::from_slice(&std::fs::read(&path).context("opening file for read")?)
            .context("loading project stack")
    }

    pub fn store(path: &Path, t: &ProjectStack) -> Result<()> {
        std::fs::write(
            &path,
            &serde_json::to_vec(&t).context("opening file for write")?,
        )
        .context("storing project stack")?;

        Ok(())
    }
}

// Adds indexing on name to project sets
pub struct RichProjectSet {
    underlying: ProjectSet,
    index_on_name: RefCell<HashMap<String, usize>>,
}

impl<'a> RichProjectSet {
    pub fn new(underlying: ProjectSet) -> Result<Self> {
        let mut instance = Self {
            underlying,
            index_on_name: RefCell::new(HashMap::new()),
        };

        Self::index(&instance.underlying, instance.index_on_name.get_mut())?;

        Ok(instance)
    }

    fn index(project_set: &ProjectSet, index_map: &mut HashMap<String, usize>) -> Result<()> {
        for (index, project) in project_set.projects.iter().enumerate() {
            if let Some(existing) = index_map.insert(project.name.clone(), index) {
                bail!(
                    "Project {:?} has the same name as project {:?}",
                    &project,
                    &project_set.projects[existing]
                );
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn reindex(&self) -> Result<()> {
        let mut new_index = HashMap::<String, usize>::new();
        Self::index(&self.underlying, &mut new_index)?;
        self.index_on_name.replace(new_index);
        Ok(())
    }

    pub fn find_index(&self, name: &str) -> Option<usize> {
        let index_on_name = self.index_on_name.borrow();
        if let Some(ix) = index_on_name.get(name) {
            return Some(*ix);
        }

        None
    }

    pub fn get(&self, name: &str) -> Option<&Project> {
        if let Some(ix) = self.find_index(name) {
            let project: &Project = &self.underlying.projects[ix];
            return Some(project);
        }

        None
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.index_on_name.borrow().contains_key(name)
    }
}

pub struct ProjectSets {
    repo_path: PathBuf,
}

impl ProjectSets {
    pub fn new(repo_path: &Path) -> Self {
        Self {
            repo_path: repo_path.to_owned(),
        }
    }

    pub fn user_directory(&self) -> PathBuf {
        self.repo_path.join(".focus")
    }

    // The projects the user has chosen
    pub fn selected_project_stack_path(&self) -> PathBuf {
        self.user_directory().join("user.stack.json")
    }

    pub fn adhoc_projects_path(&self) -> PathBuf {
        self.user_directory().join("adhoc.projects.json")
    }

    // The directory containing projects
    pub fn project_directory(&self) -> PathBuf {
        self.repo_path.join("focus").join("projects")
    }

    pub fn mandatory_project_path(&self) -> PathBuf {
        self.repo_path.join("focus").join("mandatory.projects.json")
    }

    fn project_file_filter(entry: &DirEntry) -> bool {
        if entry.path().is_dir() {
            return true;
        }

        let projects_suffix = OsString::from(".projects.json");
        let ostr = entry.path().as_os_str();
        if ostr.len() < projects_suffix.len() {
            return false;
        }

        ostr.as_bytes().ends_with(projects_suffix.as_bytes())
    }

    fn locate_project_set_files(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let mut results = Vec::<PathBuf>::new();
        let walker = WalkDir::new(path)
            .sort_by_file_name()
            .follow_links(true)
            .into_iter();
        debug!(
            project_directory = ?self.project_directory(),
            "scanning project directory",
        );

        for entry in walker.filter_entry(Self::project_file_filter) {
            match entry {
                Ok(entry) => {
                    let path = entry.path();
                    if path.is_file() {
                        results.push(path.to_owned());
                    }
                }
                Err(e) => {
                    warn!(?e, "Encountered error");
                }
            }
        }

        Ok(results)
    }

    // Return a project set containing all mandatory projects
    pub fn mandatory_projects(&self) -> Result<ProjectSet> {
        ProjectSet::load(&self.mandatory_project_path()).context("loading mandatory project set")
    }

    pub fn adhoc_projects(&self) -> Result<Option<ProjectSet>> {
        if !self.adhoc_projects_path().is_file() {
            return Ok(None);
        }

        Ok(Some(
            ProjectSet::load(&self.adhoc_projects_path()).context("loading adhoc project set")?,
        ))
    }

    pub fn storae_adhoc_project_set(&self, project_set: &ProjectSet) -> Result<()> {
        ProjectSet::store(self.adhoc_projects_path().as_path(), project_set)
            .context("storing ad-hoc project set")
    }

    // Return a project set with all available projects
    pub fn available_projects(&self) -> Result<ProjectSet> {
        let mut project = ProjectSet { projects: vec![] };

        let paths = self
            .locate_project_set_files(&self.project_directory())
            .context("locating project set files")?;

        for path in &paths {
            project.extend(
                ProjectSet::load(path)
                    .with_context(|| format!("loading project set from {}", &path.display()))?,
            );
        }

        Ok(project)
    }

    fn find_named_projects(names: &[String], set: &RichProjectSet) -> Result<Vec<Project>> {
        let mut projects = Vec::<Project>::new();

        for (index, name) in names.iter().enumerate() {
            if let Some(project) = set.get(name) {
                projects.push(project.to_owned())
            } else {
                // TODO: Provide an affordance for ignoring missing projects
                return Err(Error::new(LoadError::NotFound).context(format!(
                    "Project named '{}' (at index {}) is not present",
                    &name, index
                )));
            }
        }

        Ok(projects)
    }

    pub fn user_projects(&self) -> Result<Option<ProjectStack>> {
        let path = self.selected_project_stack_path();
        if !path.exists() {
            return Ok(None);
        }

        Ok(Some(
            ProjectStack::load(&path).context("loading user project stack")?,
        ))
    }

    // Return a project_set containing the projects a user has selected
    pub fn selected_projects(&self) -> Result<Option<ProjectSet>> {
        let project_stack = match self.user_projects()? {
            Some(stack) => stack,
            None => return Ok(None),
        };

        let indexed_available_projects = RichProjectSet::new(
            self.available_projects()
                .context("loading available projects")?,
        )?;
        let projects = Self::find_named_projects(
            &project_stack.selected_project_names,
            &indexed_available_projects,
        )
        .context("extracting selected projects from the set of all available projects")?;

        Ok(Some(ProjectSet { projects }))
    }

    // Return the computed project, namely the mandatory project and the selected project
    pub fn computed_projects(&self) -> Result<ProjectSet> {
        let mut projects = self
            .mandatory_projects()
            .context("loading mandatory projects")?;
        if let Some(adhoc_project_set) = self
            .adhoc_projects()
            .context("loading ad hoc project set")?
        {
            projects.extend(adhoc_project_set);
        }
        if let Some(selected_project_set) = self
            .selected_projects()
            .context("loading selected projects")?
        {
            projects.extend(selected_project_set);
        } else {
            warn!("No projects are selected!");
        }
        Ok(projects)
    }

    fn store_selected_projects(&self, stack: &ProjectStack) -> Result<()> {
        std::fs::create_dir_all(self.user_directory())
            .context("creating the directory to store user projects")?;
        ProjectStack::store(&self.selected_project_stack_path(), stack)
            .context("storing user project stack")
    }

    pub fn push_as_selection(&self, names: Vec<String>) -> Result<(ProjectSet, bool)> {
        // TODO: Locking
        let mut user_projects = self
            .user_projects()
            .context("loading user projects")?
            .unwrap_or_default();
        let mut selected = self.selected_projects()?.unwrap_or_default();
        let selected_indexed = RichProjectSet::new(selected.clone())?;
        let available = RichProjectSet::new(self.available_projects()?)?;
        let mut changed = false;

        for name in names {
            if selected_indexed.contains_key(&name) {
                // Already have this one
                eprintln!("{}: Skipped (already selected)", &name)
            } else if let Some(project) = available.get(&name) {
                // let name_clone = name.to_owned().to_owned();
                user_projects.selected_project_names.push(name.clone());
                selected.projects.push(project.clone());
                changed = true;
            } else {
                eprintln!("{}: Not found", &name);
                bail!("One of the requested projects was not found");
            }
        }

        if changed {
            self.store_selected_projects(&user_projects)
                .context("Failed to store the modified user project stack")?;
        }

        Ok((selected, changed))
    }

    pub fn pop(&self, count: usize) -> Result<(ProjectSet, bool)> {
        // TODO: Locking
        let mut user_projects = self
            .user_projects()
            .context("loading user projects")?
            .unwrap_or_default();
        let mut selected = self.selected_projects()?.unwrap_or_default();
        let mut changed = false;
        for _ in 0..count {
            user_projects.selected_project_names.pop();
            selected.projects.pop();
            changed = true;
        }

        self.store_selected_projects(&user_projects)
            .context("storing the modified user project stack")?;

        Ok((selected, changed))
    }

    pub fn remove(&self, names: Vec<String>) -> Result<(ProjectSet, bool)> {
        let user_projects = self
            .user_projects()
            .context("loading user project stack")?
            .unwrap_or_default();

        let mut name_set = HashSet::<String>::new();
        // let names_refs: Vec<&String> = names.iter().map(|name| name.to_owned()).collect();
        name_set.extend(names);
        let mut removals: usize = 0;
        let retained: Vec<String> = user_projects
            .selected_project_names
            .iter()
            .filter_map(|name| {
                if name_set.contains(name) {
                    removals += 1;
                    None
                } else {
                    Some(name.clone())
                }
            })
            .collect::<_>();

        if removals == 0 {
            eprintln!("No projects matched; nothing removed!");
        }

        let new_project_stack = ProjectStack {
            selected_project_names: retained,
        };

        self.store_selected_projects(&new_project_stack)
            .context("storing the modified user project stack")?;

        Ok((
            self.selected_projects()
                .context("loading selected projects")?
                .unwrap_or_default(),
            removals > 0,
        ))
    }
}

pub fn write_adhoc_project_set(sparse_repo: &Path, project_set: &ProjectSet) -> Result<()> {
    let project_sets = ProjectSets::new(sparse_repo);
    project_sets.storae_adhoc_project_set(project_set)
}
#[cfg(test)]
mod tests {
    use focus_testing::init_logging;

    use super::*;
    use anyhow::Result;
    use tempfile::{tempdir, TempDir};

    fn projects() -> Vec<Project> {
        vec![
            Project {
                name: "baseline/tools_implicit_deps".to_owned(),
                description: "".to_owned(),
                mandatory: true,
                targets: vec!["bazel://tools/implicit_deps:thrift-implicit-deps-impl".to_owned()],
            },
            Project {
                name: "baseline/scrooge_internal".to_owned(),
                description: "".to_owned(),
                mandatory: true,
                targets: vec!["bazel://tools/implicit_deps:thrift-implicit-deps-impl".to_owned()],
            },
            Project {
                name: "baseline/loglens".to_owned(),
                description: "".to_owned(),
                mandatory: true,
                targets: vec!["bazel://scrooge-internal/...".to_owned()],
            },
            Project {
                name: "projects/cdpain".to_owned(),
                description: "".to_owned(),
                mandatory: false,
                targets: vec!["bazel://workflows/examples/cdpain/...".to_owned()],
            },
        ]
    }

    fn project_set() -> ProjectSet {
        ProjectSet {
            projects: projects(),
        }
    }

    #[test]
    fn validate_no_duplicate_layer_names() -> Result<()> {
        init_logging();

        {
            let project_set = project_set();
            assert!(project_set.validate().is_ok());
        }

        {
            let mut projects = projects();
            projects.push(Project {
                name: "baseline/loglens".to_owned(),
                description: "".to_owned(),
                mandatory: false,
                targets: vec!["it doesn't matter".to_owned()],
            });
            let project_set = ProjectSet { projects };
            let e = project_set.validate().unwrap_err();
            assert_eq!("Project named 'baseline/loglens' at index 4 has the same name as existing project at index 2",e.to_string());
        }

        Ok(())
    }

    #[test]
    fn validate_no_colons() -> Result<()> {
        init_logging();

        {
            let project_set = project_set();
            assert!(project_set.validate().is_ok());
        }

        {
            let mut projects = projects();
            projects.push(Project {
                name: "beep:boop".to_owned(),
                description: "".to_owned(),
                mandatory: false,
                targets: vec!["blah".to_owned()],
            });
            let project_set = ProjectSet { projects };
            let e = project_set.validate().unwrap_err();
            assert_eq!("Project name 'beep:boop' contains a colon (:); colons are not allowed in project names",e.to_string());
        }

        Ok(())
    }

    #[test]
    fn merge() -> Result<()> {
        init_logging();

        let mut t1 = project_set();
        let t2 = ProjectSet {
            projects: vec![Project {
                name: "foo".to_owned(),
                description: "".to_owned(),
                mandatory: false,
                targets: vec!["//foo/bar/...".to_owned()],
            }],
        };

        t1.extend(t2.clone());
        assert_eq!(&t1.projects.last().unwrap(), &t2.projects.last().unwrap());
        Ok(())
    }

    #[test]
    fn remove_named_layer() -> Result<()> {
        init_logging();

        let mut project_set = project_set();
        project_set.remove_named_project("projects/cdpain")?;

        Ok(())
    }

    #[test]
    fn remove_named_layer_not_found() -> Result<()> {
        init_logging();

        let mut project_set = project_set();
        let result = project_set.remove_named_project("baseline/boo");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().root_cause().to_string(),
            RemovalError::NotFound.to_string()
        );

        Ok(())
    }

    #[test]
    fn remove_named_project_cannot_remove_mandatory_projects() -> Result<()> {
        init_logging();

        let mut project_set = project_set();
        let result = project_set.remove_named_project("baseline/loglens");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().root_cause().to_string(),
            RemovalError::Mandatory.to_string()
        );

        Ok(())
    }

    fn project_fixture(name: &str) -> ProjectSet {
        ProjectSet {
            projects: vec![Project {
                name: name.to_owned(),
                description: format!("Fixture project_set {}", name),
                mandatory: false,
                targets: vec![format!("//{}/...", name)],
            }],
        }
    }

    fn repo_fixture() -> Result<(TempDir, ProjectSets)> {
        let dir = tempdir().context("making a temporary directory")?;
        let path = dir.path().join("test_repo");
        let t = ProjectSets::new(&path);
        let project_dir = t.project_directory();
        std::fs::create_dir_all(&project_dir).context("creating project dir")?;

        let random_file_path = project_dir.join("whatever.json");
        std::fs::write(&random_file_path, b"{}").context("writing random file")?;

        let builtins_layer = project_set();
        let builtins_path = project_dir.join("builtins.projects.json");
        ProjectSet::store(&builtins_path, &builtins_layer).context("storing builtins_layer")?;

        Ok((dir, t))
    }

    #[test]
    fn available_projects() -> Result<()> {
        init_logging();

        let (_tdir, t) = repo_fixture().context("building repo fixture")?;
        let project_dir = t.project_directory();

        let my_project_path = project_dir.join("my_project.projects.json");
        let my_project = project_fixture("my_project");
        ProjectSet::store(&my_project_path, &my_project).context("storing my_project")?;

        let cat = t
            .available_projects()
            .context("reading available projects")?;
        assert_eq!(cat.projects.len(), 5);

        Ok(())
    }

    #[test]
    fn optional_projects() -> Result<()> {
        init_logging();
        let ls = vec![
            Project {
                name: "a".to_owned(),
                description: "".to_owned(),
                targets: vec!["//a/...".to_owned()],
                mandatory: true,
            },
            Project {
                name: "b".to_owned(),
                description: "".to_owned(),
                targets: vec!["//b/...ı".to_owned()],
                mandatory: false,
            },
        ];
        let t = ProjectSet { projects: ls };

        let projects = t.optional_projects()?;
        assert_eq!(projects.len(), 1);
        assert_eq!(projects.last().unwrap().name, "b");

        Ok(())
    }

    #[allow(dead_code)]
    fn selected_projects() -> Result<()> {
        init_logging();

        let (_tdir, t) = repo_fixture().context("building repo fixture")?;
        assert!(t.selected_projects().unwrap().is_none());

        Ok(())
    }
}
