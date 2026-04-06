use anyhow::Result;
use common::settings::CONFIG_FILE;
//use fastembed::{EmbeddingModel, InitOptions, ModelTrait, TextEmbedding};
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
    // info!(
    //     "model info: {:?}",
    //     fastembed::EmbeddingModel::get_model_info(&EmbeddingModel::AllMiniLML6V2)
    // );
    let embedder = create_default_embedder();

    //    let embedder = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))?;
    // ---------------------------
    // Connect to Qdrant
    // ---------------------------
    let client = Qdrant::from_url("http://localhost:6334").build()?;

    search::perform_search(config, embedder, client).await

    // // ---------------------------
    // // Embed query
    // // ---------------------------
    // let embedding = embedder.embed(vec![config.query.clone()], None)?;
    // let query_vector = embedding[0].clone();

    // // ---------------------------
    // // Build optional filter
    // // ---------------------------
    // let filter = config
    //     .source
    //     .map(|source| Filter::must([Condition::matches_text("source", source)]));

    // // ---------------------------
    // // Search
    // // ---------------------------
    // let query_request = match filter {
    //     None => QueryPointsBuilder::new(config.collection) // Collection name
    //         .query(query_vector) // Query vector
    //         .limit(config.limit) // Search limit, number of results to return
    //         .with_payload(true),
    //     Some(filter) => QueryPointsBuilder::new(config.collection) // Collection name
    //         .query(query_vector) // Query vector
    //         .limit(config.limit) // Search limit, number of results to return
    //         .filter(filter)
    //         .with_payload(true),
    // };

    // let results = client.query(query_request).await?;
    // // ---------------------------
    // // Display results
    // // ---------------------------

    // for (i, point) in results.result.iter().enumerate() {
    //     let payload = &point.payload;

    //     let text = payload
    //         .get("text")
    //         .and_then(|v| v.as_str())
    //         .map(|v| v.as_str())
    //         .unwrap_or("");

    //     let source = payload
    //         .get("source")
    //         .and_then(|v| v.as_str())
    //         .map(|v| v.as_str())
    //         .unwrap_or("");

    //     info!(
    //         result = i + 1,
    //         score = point.score,
    //         source,
    //         text = truncate(text, 200),
    //         ?payload
    //     );
    // }

    // Ok(())
}
