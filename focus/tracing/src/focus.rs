// module to collect tracing setup and config for the focus app itself

use std::fs::OpenOptions;
use std::io::{self, BufWriter};
use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing::dispatcher::DefaultGuard;
use tracing::metadata::LevelFilter;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_error::ErrorLayer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{self, util::SubscriberInitExt, EnvFilter};

#[derive(Debug)]
pub enum GuardWrapper {
    WorkerGuard(WorkerGuard),
    DefaultGuard(DefaultGuard),
}

#[derive(Debug, Default)]
/// opaque struct for returning tracing WorkerGuard instances to main
pub struct Guard {
    _inner: Vec<GuardWrapper>,
}

#[derive(Debug, Default)]
pub struct TracingOpts {
    pub is_tty: bool,
    pub no_color: bool,
    pub log_dir: Option<PathBuf>,
}

const LOG_FILE_NAME: &str = "focus.log";

pub fn init_tracing(opts: TracingOpts) -> Result<Guard> {
    let TracingOpts {
        is_tty,
        no_color,
        log_dir,
    } = opts;

    let use_color = is_tty && !no_color;

    let log_dir = match log_dir {
        Some(dir) => dir,
        None => super::log_dir().context("could not determine default log dir")?,
    };

    std::fs::create_dir_all(&log_dir)?;

    let log_path = log_dir.join(LOG_FILE_NAME);

    // TODO: figure out log file rotation
    let (log_file_writer, log_file_guard) = tracing_appender::non_blocking(BufWriter::new(
        OpenOptions::new()
            .append(true)
            .create(true)
            .open(&log_path)
            .context("failed to open log file")?,
    ));

    let (stderr_writer, stderr_guard) = tracing_appender::non_blocking(io::stderr());

    tracing_subscriber::registry()
        .with(ErrorLayer::default())
        .with(EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
        ))
        .with(
            Targets::new()
                .with_targets(vec![
                    ("serde_xml_rs", LevelFilter::INFO),
                    (
                        "focus_internals::coordinate_resolver::bazel_resolver",
                        LevelFilter::INFO,
                    ),
                ])
                .with_default(LevelFilter::TRACE),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                .with_target(true)
                .with_writer(stderr_writer)
                .with_ansi(use_color),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                .with_target(false)
                .with_writer(log_file_writer),
        )
        .try_init()?;

    Ok(Guard {
        _inner: vec![
            GuardWrapper::WorkerGuard(stderr_guard),
            GuardWrapper::WorkerGuard(log_file_guard),
        ],
    })
}
