use anyhow::Result;
use toml::Map;

#[derice(Debug)]
pub struct Layer {
    coordinates: Vec<String>,
}

#[derive(Debug)]
pub struct Topology {
    named_layers: Map<String, Layer>,
}
