use std::path::Path;

use anyhow::{Context, Result};

use crate::model::{self, LayerSets};

pub fn available(repo: &Path) -> Result<()> {
    let layer_sets = model::LayerSets::new(&repo);
    let set = &layer_sets.available_layers()?;
    for layer in set.layers() {
        println!("{}", layer);
    }

    Ok(())
}

pub fn selected(repo: &Path) -> Result<()> {
    let sets = LayerSets::new(&repo);

    if let Some(selected) = sets.selected_layers().context("loading selected layers")? {
        // TODO: Extract printing and re-use
        if selected.layers().is_empty() {
            eprintln!("No layers are selected, but a stack exists");
            return Ok(());
        }
        for (index, layer) in selected.layers().iter().enumerate() {
            println!("{}: {}", index, layer);
        }
    } else {
        eprintln!("No layers are selected, and no stack exists");
    }

    if let Ok(Some(adhoc_layers)) = sets.adhoc_layers() {
        for layer in adhoc_layers.layers() {
            eprintln!("[ad-hoc]: {}", layer);
        }
    }

    Ok(())
}

pub fn push(repo: &Path, names: Vec<String>) -> Result<()> {
    // Push a layer
    let sets = model::LayerSets::new(&repo);

    let new_selection = sets.push_as_selection(names).context("pushing layer")?;

    if new_selection.layers().is_empty() {
        eprintln!("The layer stack is empty!");
    } else {
        for (index, layer) in new_selection.layers().iter().enumerate() {
            println!("{}: {}", index, layer)
        }
    }

    Ok(())
}

pub fn pop(repo: &Path, count: usize) -> Result<()> {
    // Pop a layer
    let sets = model::LayerSets::new(&repo);

    let new_selection = sets.pop(count).context("popping layers")?;

    if new_selection.layers().is_empty() {
        eprintln!("The layer stack is empty!");
    } else {
        for (index, layer) in new_selection.layers().iter().enumerate() {
            println!("{}: {}", index, layer)
        }
    }

    Ok(())
}

pub fn remove(repo: &Path, names: Vec<String>) -> Result<()> {
    // Remove a layer
    let sets = model::LayerSets::new(&repo);

    let new_selection = sets.remove(names).context("removing layers")?;

    if new_selection.layers().is_empty() {
        eprintln!("The layer stack is empty!");
    } else {
        for (index, layer) in new_selection.layers().iter().enumerate() {
            println!("{}: {}", index, layer)
        }
    }

    Ok(())
}