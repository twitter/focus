use std::{path::PathBuf, sync::Once};

use anyhow::{Context, Result};

use tracing_error::ErrorLayer;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

pub mod scratch_git_repo;

pub fn init_logging() {
    static START: Once = Once::new();
    START.call_once(|| {
        // TODO: De-dup this egregious rip-off of `focus_commands::init_logging`.
        let is_tty = termion::is_tty(&std::io::stdout());
        let nocolor_requested = std::env::var_os("NO_COLOR").is_some(); // see https://no-color.org/
        let use_color = is_tty && !nocolor_requested;
        tracing_subscriber::registry()
            .with(ErrorLayer::default())
            .with(EnvFilter::new(
                std::env::var("RUST_LOG").unwrap_or_else(|_| "focus=debug".to_string()),
            ))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                    .with_target(false)
                    .with_ansi(use_color),
            )
            .try_init()
            .unwrap();
    });
}

pub fn fixture_dir() -> Result<PathBuf> {
    Ok(std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .context("Reading CARGO_MANIFEST_DIR")?
        .parent()
        .unwrap()
        .join("fixtures"))
}
