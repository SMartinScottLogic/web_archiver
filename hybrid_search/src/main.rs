use anyhow::Result;
use clap::Parser;
use fastembed::{EmbeddingModel, InitOptions, ModelTrait, TextEmbedding};
use qdrant_client::{
    qdrant::{Condition, Filter, QueryPointsBuilder},
    Qdrant,
};

#[derive(Parser)]
struct Args {
    /// Query text
    query: Option<String>,

    /// Optional source filter
    #[arg(long)]
    source: Option<String>,

    /// Number of results wanted
    #[arg(long, short, default_value = "5")]
    limit: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // ---------------------------
    // Init embedder
    // ---------------------------
    println!(
        "model info: {:?}",
        fastembed::EmbeddingModel::get_model_info(&EmbeddingModel::AllMiniLML6V2)
    );
    let mut embedder = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))?;
    // ---------------------------
    // Connect to Qdrant
    // ---------------------------
    let client = Qdrant::from_url("http://localhost:6334").build()?;

    let collection = "articles";

    // ---------------------------
    // Embed query
    // ---------------------------
    let embedding = embedder.embed(
        vec![args
            .query
            .unwrap_or("son fucks mum in car".to_string())
            .clone()],
        None,
    )?;
    let query_vector = embedding[0].clone();

    // ---------------------------
    // Build optional filter
    // ---------------------------
    let filter = args
        .source
        .map(|source| Filter::must([Condition::matches_text("source", source)]));

    // ---------------------------
    // Search
    // ---------------------------
    let query_request = match filter {
        None => QueryPointsBuilder::new(collection) // Collection name
            .query(query_vector) // Query vector
            .limit(args.limit) // Search limit, number of results to return
            .with_payload(true),
        Some(filter) => QueryPointsBuilder::new(collection) // Collection name
            .query(query_vector) // Query vector
            .limit(args.limit) // Search limit, number of results to return
            .filter(filter)
            .with_payload(true),
    };

    let results = client.query(query_request).await?;
    // ---------------------------
    // Display results
    // ---------------------------
    println!("\nTop results:\n");

    for (i, point) in results.result.iter().enumerate() {
        let payload = &point.payload;

        let text = payload
            .get("text")
            .and_then(|v| v.as_str())
            .map(|v| v.as_str())
            .unwrap_or("");

        let source = payload
            .get("source")
            .and_then(|v| v.as_str())
            .map(|v| v.as_str())
            .unwrap_or("");

        println!("Result {}:", i + 1);
        println!("Score: {:.4}", point.score);
        println!("Source: {}", source);
        println!("Text: {}", truncate(text, 200));
        println!("Payload: {:?}\n", payload);
    }

    Ok(())
}

// ---------------------------
// Helper
// ---------------------------
fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max])
    } else {
        s.to_string()
    }
}
