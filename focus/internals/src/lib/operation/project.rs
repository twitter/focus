use std::path::Path;

use anyhow::{Context, Result};

use crate::model::project::ProjectSets;

use std::collections::HashSet;

pub fn available(repo: &Path) -> Result<bool> {
    let project_sets = ProjectSets::new(repo);
    let set = &project_sets.available_projects()?;
    for project in set.projects() {
        println!("{}", project);
    }

    Ok(false)
}

pub fn selected_layer_names(repo: &Path) -> Result<HashSet<String>> {
    let mut results = HashSet::<String>::new();
    let sets = ProjectSets::new(repo);
    if let Some(selected) = sets
        .selected_projects()
        .context("loading selected projects")?
    {
        results.extend(
            selected
                .projects()
                .iter()
                .map(|project| project.name().to_owned()),
        );
    }
    Ok(results)
}

pub fn list(repo: &Path) -> Result<bool> {
    let sets = ProjectSets::new(repo);

    if let Some(selected) = sets
        .selected_projects()
        .context("loading selected projects")?
    {
        if selected.projects().is_empty() {
            eprintln!("No projects are selected!");
            return Ok(false);
        }
        for (index, project) in selected.projects().iter().enumerate() {
            println!("{}: {}", index, project);
        }
    } else {
        eprintln!("No projects are selected!");
    }

    if let Ok(Some(adhoc_project_set)) = sets.adhoc_projects() {
        for project in adhoc_project_set.projects() {
            eprintln!("[ad-hoc]: {}", project);
        }
    }

    Ok(false)
}

pub fn push(repo: &Path, names: Vec<String>) -> Result<bool> {
    // Push a project
    let sets = ProjectSets::new(repo);

    let (new_selection, changed) = sets
        .push_as_selection(names)
        .context("pushing projects onto the stack of selected projects")?;

    if new_selection.projects().is_empty() {
        eprintln!("No projects are selected!");
    } else {
        for (index, project) in new_selection.projects().iter().enumerate() {
            println!("{}: {}", index, project)
        }
    }

    Ok(changed)
}

pub fn pop(repo: &Path, count: usize) -> Result<bool> {
    // Pop a project
    let sets = ProjectSets::new(repo);

    let (new_selection, changed) = sets
        .pop(count)
        .context("popping projects from the stack of selected projects")?;

    if new_selection.projects().is_empty() {
        eprintln!("No projects are selected!");
    } else {
        for (index, project) in new_selection.projects().iter().enumerate() {
            println!("{}: {}", index, project)
        }
    }

    Ok(changed)
}

pub fn remove(repo: &Path, names: Vec<String>) -> Result<bool> {
    // Remove a project
    let sets = ProjectSets::new(repo);

    let (new_selection, changed) = sets.remove(names).context("removing named projects")?;

    if new_selection.projects().is_empty() {
        eprintln!("No projects are selected!");
    } else {
        for (index, project) in new_selection.projects().iter().enumerate() {
            println!("{}: {}", index, project)
        }
    }

    Ok(changed)
}
