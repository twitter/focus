use std::path::Path;

use anyhow::Result;

use crate::model;

pub fn run(repo: &Path) -> Result<()> {
    let layer_sets = model::LayerSets::new(&repo);
    let set = &layer_sets.available_layers()?;
    for layer in set.layers() {
        println!("{}", layer);
    }

    Ok(())
}
