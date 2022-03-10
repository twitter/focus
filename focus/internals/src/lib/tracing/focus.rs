// module to collect tracing setup and config for the focus app itself

use std::io;

use anyhow::Result;
use tracing::dispatcher::DefaultGuard;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_error::ErrorLayer;
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

impl From<WorkerGuard> for Guard {
    fn from(wg: WorkerGuard) -> Self {
        Guard {
            _inner: vec![GuardWrapper::WorkerGuard(wg)],
        }
    }
}

#[derive(Debug, Default)]
pub struct TracingOpts {
    pub is_tty: bool,
    pub nocolor_requested: bool,
}

pub fn init_tracing(opts: TracingOpts) -> Result<Guard> {
    let TracingOpts {
        is_tty,
        nocolor_requested,
    } = opts;
    let use_color = is_tty && !nocolor_requested;
    let (non_blocking, guard) = tracing_appender::non_blocking(io::stderr());

    tracing_subscriber::registry()
        .with(ErrorLayer::default())
        .with(EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
        ))
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                .with_target(false)
                .with_writer(non_blocking)
                .with_ansi(use_color),
        )
        .try_init()?;

    Ok(Guard::from(guard))
}
