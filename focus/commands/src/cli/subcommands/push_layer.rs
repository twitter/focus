use std::path::Path;

use anyhow::{Context, Result};

use crate::model;

pub fn run(repo: &Path, names: Vec<String>) -> Result<()> {
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
