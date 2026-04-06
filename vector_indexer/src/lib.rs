pub mod chunk_markdown;
pub mod settings;
pub mod vector_db;

use std::{fs, path::Path};

use common::{historical::HistoricalPage, page::PageReader, types::ExtractedPage};
use map_macro::hash_map;
use qdrant_client::qdrant::{PointStruct, UpsertPointsBuilder};
use serde_json::json;
use tracing::{error, info, warn};
use uuid::Uuid;
use vector_common::Embedder;
use walkdir::WalkDir;

use crate::vector_db::VectorDb;

const CHUNK_SIZE: usize = 500;
const OVERLAP: usize = 96;

fn read_page(path: &Path) -> anyhow::Result<Box<dyn PageReader>> {
    let text = fs::read_to_string(path)?;

    if let Ok(content) = serde_json::from_str::<ExtractedPage>(&text) {
        return Ok(Box::new(content));
    }
    if let Ok(content) = serde_json::from_str::<HistoricalPage>(&text) {
        return Ok(Box::new(content));
    }
    Err(anyhow::Error::msg(format!(
        "Failed to parse {}",
        path.display()
    )))
}

fn current_content(path: &Path) -> Option<String> {
    let content = match read_page(path) {
        Err(e) => {
            error!(
                "Failed to convert content of {} into PageReader: {:?}",
                path.display(),
                e
            );
            return None;
        }
        Ok(r) => r,
    };

    match content.current() {
        None => {
            warn!("Processing {}: No content", path.display());
            None
        }
        Some(r) => match &r.content_markdown {
            common::historical::HistoricalContentType::None => {
                warn!("Processing {}: Empty content", path.display());
                None
            }
            common::historical::HistoricalContentType::Literal(t) if t.is_empty() => {
                warn!("Processing {}: Empty content", path.display());
                None
            }
            common::historical::HistoricalContentType::Literal(t) if t.len() < 5_000 => {
                warn!(
                    "Processing {}: Too little content {} < 5,000",
                    path.display(),
                    t.len()
                );
                None
            }
            common::historical::HistoricalContentType::Literal(text) => {
                info!("Processing {}", path.display());
                Some(text.to_owned())
            }
            common::historical::HistoricalContentType::Delta(_) => {
                error!(
                    "Processing {}: Invalid content (current cannot be a delta)",
                    path.display()
                );
                None
            }
        },
    }
}

pub async fn populate_vector_db(
    embedder: &mut impl Embedder,
    client: &impl VectorDb,
    collection: &str,
    root: &str,
) -> anyhow::Result<()> {
    // ---------------------------
    // Iterate over documents
    // ---------------------------
    for entry in WalkDir::new(root) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();

        let markdown = match current_content(path) {
            Some(text) => text,
            None => continue,
        };

        let chunks = chunk_markdown::chunk_markdown(&markdown, CHUNK_SIZE, OVERLAP);

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

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::vector_db::MockVectorDb;

    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;
    use tracing_test::traced_test;
    use vector_common::MockEmbedder;

    #[test]
    fn read_page_fails_on_invalid_json() {
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), "not json").unwrap();

        let result = read_page(file.path());

        assert!(result.is_err());
    }

    #[test]
    fn current_content_returns_none_if_read_fails() {
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), "invalid json").unwrap();

        let result = current_content(file.path());

        assert!(result.is_none());
    }

    #[test]
    fn current_content_rejects_small_content() {
        let file = NamedTempFile::new().unwrap();

        let json = serde_json::json!({
            "content_markdown": {
                "Literal": "short text"
            }
        });

        fs::write(file.path(), json.to_string()).unwrap();

        let result = current_content(file.path());

        assert!(result.is_none());
    }

    #[test]
    #[traced_test]
    fn current_content_accepts_large_content() {
        let file = NamedTempFile::new().unwrap();

        let big_text = "a".repeat(6000);

        let json = serde_json::json!({
            "current": {
                "task": {
                    "url_id": 0,
                    "url": "example.com",
                    "depth": 0,
                    "priority": 0
                },
                "content_markdown": {
                    "Literal": big_text
                },
                "links": []
            },
            "url": "example.com",
            "historical_snapshots": [],
            "all_links": []
        });

        fs::write(file.path(), json.to_string()).unwrap();
        println!("file: {}", file.path().display());
        let result = current_content(file.path());

        assert!(result.is_some());
    }

    #[tokio::test]
    async fn populate_vector_db_processes_files() {
        let dir = tempfile::tempdir().unwrap();

        let file_path = dir.path().join("test.json");

        let big_text = "a".repeat(6000);

        let json = serde_json::json!({
            "content_markdown": {
                "Literal": big_text
            }
        });

        fs::write(&file_path, json.to_string()).unwrap();

        let db = MockVectorDb::new();
        let mut embedder = MockEmbedder::new();

        populate_vector_db(&mut embedder, &db, "test", dir.path().to_str().unwrap())
            .await
            .unwrap();
    }
}
