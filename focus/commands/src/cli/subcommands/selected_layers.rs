use std::path::Path;

use anyhow::{Context, Result};


use crate::model::LayerSets;

pub fn run(repo: &Path) -> Result<()> {
    let sets = LayerSets::new(&repo);
    if let Some(selected) = sets.selected_layers().context("loading selected layers")? {
        for layer in selected.layers() {
            println!("{}", layer);
        }
    } else {
        eprintln!("No layers are selected");
    }
    
    Ok(())
}
