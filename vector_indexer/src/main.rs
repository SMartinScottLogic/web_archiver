use anyhow::Result;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use map_macro::hash_map;
use qdrant_client::{
    Qdrant,
    qdrant::{
        CreateCollectionBuilder, Distance, PointStruct, ScalarQuantizationBuilder,
        SearchParamsBuilder, SearchPointsBuilder, UpsertPointsBuilder, VectorParamsBuilder,
    },
};
use serde_json::json;
use std::fs;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};
use uuid::Uuid;
use walkdir::WalkDir;

const CHUNK_SIZE: usize = 500;
const OVERLAP: usize = 96;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging ---
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_thread_ids(true) // show thread IDs
        .with_thread_names(true) // show thread names
        .with_span_events(FmtSpan::NONE)
        .init();

    // ---------------------------
    // Initialize embedding model
    // ---------------------------
    let mut embedder = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))?;

    // ---------------------------
    // Connect to Qdrant
    // ---------------------------
    let client = Qdrant::from_url("http://localhost:6334").build()?;

    let collection = "literotica";
    if !client.collection_exists(collection).await? {
        client
            .create_collection(
                CreateCollectionBuilder::new(collection)
                    .vectors_config(VectorParamsBuilder::new(384, Distance::Cosine))
                    .quantization_config(ScalarQuantizationBuilder::default()),
            )
            .await?;
    }
    // Create collection if it doesn't exist
    let query_embedding = embedder.embed(vec!["fuck mom on back seat"], None)?;

    match client
        .search_points(
            SearchPointsBuilder::new(collection, query_embedding[0].clone(), 10)
                .with_payload(true)
                .params(SearchParamsBuilder::default().hnsw_ef(128).exact(false)),
        )
        .await
    {
        Err(e) => error!(error = ?e, "search failed"),
        Ok(r) => {
            info!("search result: {:#?}", r);
            //return Ok(());
        }
    }

    // ---------------------------
    // Iterate over documents
    // ---------------------------
    for entry in WalkDir::new("archive") {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let text = fs::read_to_string(path)?;
        let content: common::types::ExtractedPage = match serde_json::from_str(&text) {
            Err(e) => {
                error!(
                    "Failed to convert content of {} into ExtractedPage: {:?}",
                    path.display(),
                    e
                );
                continue;
            }
            Ok(r) => r,
        };

        let markdown = match content.content_markdown {
            None => {
                warn!("Processing {}: No content", path.display());
                continue;
            }
            Some(r) if r.is_empty() => {
                warn!("Processing {}: Empty content", path.display());
                continue;
            }
            Some(r) if r.len() < 5_000 => {
                warn!(
                    "Processing {}: Too little content {} < 5,000",
                    path.display(),
                    r.len()
                );
                continue;
            }
            Some(r) => {
                info!("Processing {}", path.display());
                r
            }
        };

        //let chunks = chunk_text(&markdown, CHUNK_SIZE);

        let chunks = chunk_markdown(&markdown, CHUNK_SIZE, OVERLAP);

        let text_chunks = chunks
            .iter()
            .map(|chunk| chunk.text.to_owned())
            .collect::<Vec<_>>();

        let embeddings = embedder.embed(text_chunks, None)?;

        let mut points = Vec::new();

        for (chunk_id, (chunk, embedding)) in chunks.into_iter().zip(embeddings).enumerate() {
            info!(chunk_id, path = ?path, total_len = markdown.len(), "upsert");
            let payload = hash_map!(
                "text".to_string() => json!(chunk.text),
                "chunk_id".to_string() => json!(chunk_id),
                "source".to_string() => json!(path.display().to_string())
            );

            let point = PointStruct::new(Uuid::new_v4().to_string(), embedding, payload);

            points.push(point);
        }

        if let Err(e) = client
            .upsert_points(UpsertPointsBuilder::new(collection, points))
            .await
        {
            error!(error = ?e, "Failed to upsert points");
        }
    }

    println!("Ingestion complete.");

    Ok(())
}

// fn chunk_text(text: &str, chunk_size: usize) -> Vec<String> {
//     let words: Vec<&str> = text.split_whitespace().collect();

//     let mut chunks = Vec::new();
//     let mut current = Vec::new();

//     for word in words {
//         current.push(word);

//         if current.len() >= chunk_size {
//             chunks.push(current.join(" "));
//             current.clear();
//         }
//     }

//     if !current.is_empty() {
//         chunks.push(current.join(" "));
//     }

//     chunks
// }

// ---------------------------
// Markdown-aware chunking
// ---------------------------

struct Chunk {
    text: String,
}

fn chunk_markdown(text: &str, chunk_size: usize, overlap: usize) -> Vec<Chunk> {
    let blocks = split_markdown_blocks(text);

    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut current_tokens = 0;

    for block in blocks {
        let tokens = estimate_tokens(&block);

        // If adding this block exceeds chunk size → flush
        if current_tokens + tokens > chunk_size && !current.is_empty() {
            let chunk_text = current.join("\n\n");

            chunks.push(Chunk {
                text: chunk_text.clone(),
            });

            // Handle overlap
            let overlap_text = take_overlap(&chunk_text, overlap);
            current = vec![overlap_text];
            current_tokens = estimate_tokens(&current[0]);
        }

        current.push(block);
        current_tokens += tokens;
    }

    // अंतिम chunk
    if !current.is_empty() {
        chunks.push(Chunk {
            text: current.join("\n\n"),
        });
    }

    chunks
}

fn split_markdown_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = Vec::new();

    for line in text.lines() {
        // New section on headings
        if line.starts_with('#') && !current.is_empty() {
            blocks.push(current.join("\n"));
            current.clear();
        }

        // Paragraph break
        if line.trim().is_empty() && !current.is_empty() {
            blocks.push(current.join("\n"));
            current.clear();
            continue;
        }

        current.push(line.to_string());
    }

    if !current.is_empty() {
        blocks.push(current.join("\n"));
    }

    blocks
}

fn estimate_tokens(text: &str) -> usize {
    text.split_whitespace().count()
}

fn take_overlap(text: &str, overlap_tokens: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();

    let start = if words.len() > overlap_tokens {
        words.len() - overlap_tokens
    } else {
        0
    };

    words[start..].join(" ")
}
