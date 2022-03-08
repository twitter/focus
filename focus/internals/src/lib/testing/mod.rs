use std::{sync::Once};

use tracing_subscriber::FmtSubscriber;

pub mod scratch_git_repo;

pub fn init_logging() {
    static START: Once = Once::new();
    START.call_once(|| {
        let subscriber = FmtSubscriber::builder().finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("Setting the default tracing subscriber failed");
    });
}
