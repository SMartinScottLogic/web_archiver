use anyhow::Result;
use common::settings::CONFIG_FILE;
use qdrant_client::Qdrant;
use tracing::{info, level_filters::LevelFilter};

use settings::Config;
use tracing_subscriber::{fmt::format::FmtSpan, EnvFilter};
use vector_common::create_default_embedder;

mod search;
mod settings;

fn setup_logging() {
    // Initialize logging
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_span_events(FmtSpan::NONE)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_logging();

    let config =
        Config::file(CONFIG_FILE).unwrap_or_else(|_| panic!("Failed to load {}", CONFIG_FILE));

    info!("config: {:?}", config);

    // ---------------------------
    // Init embedder
    // ---------------------------
    let embedder = create_default_embedder();

    // ---------------------------
    // Connect to Qdrant
    // ---------------------------
    let client = Qdrant::from_url("http://localhost:6334").build()?;

    search::perform_search(config, embedder, client).await
}
