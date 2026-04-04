use anyhow::Result;
use common::settings::CONFIG_FILE;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use qdrant_client::Qdrant;
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};
use vector_indexer::{
    populate_vector_db,
    settings::Config,
    vector_db::{perform_query, setup_vector_db},
};

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
    // Initialize embedding model
    // ---------------------------
    let mut embedder = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))?;
    let client = Qdrant::from_url("http://localhost:6334").build()?;
    setup_vector_db(&client, &config.collection).await?;

    perform_query(&mut embedder, &client, &config.collection, "sample doc").await?;

    populate_vector_db(
        &mut embedder,
        &client,
        &config.collection,
        &config.archive_dir,
    )
    .await?;

    println!("Ingestion complete.");

    Ok(())
}
