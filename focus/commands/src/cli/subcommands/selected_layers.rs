use std::path::Path;

use anyhow::{Context, Result};


use crate::model::LayerSets;

pub fn run(repo: &Path) -> Result<()> {
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
    
    Ok(())
}
