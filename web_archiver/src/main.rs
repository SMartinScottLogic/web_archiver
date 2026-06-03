use common::DefaultArchiver;
use tracing::level_filters::LevelFilter;

mod extractor;
mod fetcher;
mod frontier;
mod json;
mod mail;
mod settings;
mod system;

use tracing::info;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

use crate::frontier::db::frontier::FrontierDb;
use crate::settings::load_config;

/// Initialize logging ---
fn setup_logging() {
    // Initialize logging
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy()
        .add_directive("html5ever::serialize=error".parse().unwrap());
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_span_events(FmtSpan::NONE)
        .init();
}

#[tokio::main]
async fn main() {
    setup_logging();
    info!("Starting Web Archiver (Migrated)");

    load_config();

    let _ = system::run_system::<DefaultArchiver, FrontierDb>().await;

    info!("Shutting down");
}
