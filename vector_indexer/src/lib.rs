pub mod chunk_markdown;
pub mod settings;
pub mod vector_db;

use std::{fs, path::Path};

use common::{historical::HistoricalPage, page::PageReader};
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

fn read_page<T>(path: &Path) -> anyhow::Result<T>
where
    T: PageReader + for<'a> serde::Deserialize<'a>,
{
    let text = fs::read_to_string(path)?;

    if let Ok(content) = serde_json::from_str::<T>(&text) {
        return Ok(content);
    }
    Err(anyhow::Error::msg(format!(
        "Failed to parse {}",
        path.display()
    )))
}

fn current_content<T>(path: &Path) -> Option<String>
where
    T: PageReader + for<'a> serde::Deserialize<'a>,
{
    let content = match read_page::<T>(path) {
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

    match content.current().to_owned() {
        None => {
            warn!("Processing {}: No content", path.display());
            None
        }
        Some(mut r) => {
            r.content_markdown.sort_by_cached_key(|c| c.page);
            let mut content = String::new();
            for page in r.content_markdown {
                match page.content {
                    common::historical::HistoricalContentType::None => {
                        warn!("Processing {}: Empty content", path.display());
                        return None;
                    }
                    common::historical::HistoricalContentType::Literal(t) if t.is_empty() => {
                        warn!("Processing {}: Empty content", path.display());
                        return None;
                    }
                    common::historical::HistoricalContentType::Literal(t) if t.len() < 5_000 => {
                        warn!(
                            "Processing {}: Too little content {} < 5,000",
                            path.display(),
                            t.len()
                        );
                        return None;
                    }
                    common::historical::HistoricalContentType::Literal(text) => {
                        info!("Processing {}", path.display());
                        content.push_str(&text);
                    }
                    common::historical::HistoricalContentType::Delta(_) => {
                        error!(
                            "Processing {}: Invalid content (current cannot be a delta)",
                            path.display()
                        );
                        return None;
                    }
                };
            }
            Some(content)
        }
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

        let markdown = match current_content::<HistoricalPage>(path) {
            Some(text) => text,
            None => continue,
        };

        let points = process_markdown(path, embedder, &markdown)?;

        if let Err(e) = client
            .upsert_points(UpsertPointsBuilder::new(collection, points))
            .await
        {
            error!(error = ?e, "Failed to upsert points");
        }
    }

    Ok(())
}

fn process_markdown(
    path: &Path,
    embedder: &mut impl Embedder,
    markdown: &str,
) -> anyhow::Result<Vec<PointStruct>> {
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
    Ok(points)
}

#[cfg(test)]
mod tests {
    use crate::vector_db::MockVectorDb;

    use super::*;
    use common::{
        historical::{HistoricalContent, HistoricalSnapshot},
        types::{FetchTask, Priority},
    };
    use qdrant_client::qdrant::{PointsOperationResponse, UpdateStatus};
    use std::{
        collections::{HashSet, VecDeque},
        fs,
    };
    use tempfile::NamedTempFile;
    use tracing_test::traced_test;
    use vector_common::MockEmbedder;

    #[test]
    fn read_page_fails_on_invalid_json() {
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), "not json").unwrap();

        let result = read_page::<HistoricalPage>(file.path());

        assert!(result.is_err());
    }

    #[test]
    fn current_content_returns_none_if_read_fails() {
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), "invalid json").unwrap();

        let result = current_content::<HistoricalPage>(file.path());

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

        let result = current_content::<HistoricalPage>(file.path());

        assert!(result.is_none());
    }

    #[test]
    #[traced_test]
    fn current_content_accepts_large_content() {
        let file = NamedTempFile::new().unwrap();

        let w = common::historical::HistoricalPage {
            task: common::types::FetchTask {
                article_id: 0,
                url_id: 0,
                url: "example.com".to_string(),
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
            },
            current: Some(common::historical::HistoricalSnapshot {
                links: std::collections::HashSet::new(),
                metadata: None,
                content_markdown: vec![common::historical::HistoricalContent {
                    page: 1,
                    content: common::historical::HistoricalContentType::Literal(
                        "big_text".to_string(),
                    ),
                }],
            }),
            historical_snapshots: VecDeque::new(),
            all_links: std::collections::HashSet::new(),
        };

        println!("full: '{}'", serde_json::to_string(&w).unwrap());

        let big_text = "a".repeat(6000);

        let json = serde_json::json!({
            "task": {
                "url_id": 0,
                "url": "example.com",
                "depth": 0,
                "priority": 0
            },
            "current": {
                "content_markdown": [{
                    "content": {
                        "Literal": big_text
                    },
                    "page": 1
                }],
                "links": []
            },
            "url": "example.com",
            "historical_snapshots": [],
            "all_links": []
        });

        fs::write(file.path(), json.to_string()).unwrap();
        println!("file: {}", file.path().display());
        let result = current_content::<HistoricalPage>(file.path());

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

    #[test]
    fn current_content_rejects_none_content() {
        let file = NamedTempFile::new().unwrap();

        let json = serde_json::json!({
            "task": {},
            "current": {
                "content_markdown": [{
                    "content": "None",
                    "page": 1
                }],
                "links": []
            },
            "historical_snapshots": [],
            "all_links": []
        });

        fs::write(file.path(), json.to_string()).unwrap();

        let result = current_content::<HistoricalPage>(file.path());
        assert!(result.is_none());
    }

    #[test]
    fn current_content_rejects_empty_literal() {
        let file = NamedTempFile::new().unwrap();

        let json = serde_json::json!({
            "task": {},
            "current": {
                "content_markdown": [{
                    "content": { "Literal": "" },
                    "page": 1
                }],
                "links": []
            },
            "historical_snapshots": [],
            "all_links": []
        });

        fs::write(file.path(), json.to_string()).unwrap();

        let result = current_content::<HistoricalPage>(file.path());
        assert!(result.is_none());
    }

    #[test]
    fn current_content_rejects_delta_content() {
        let file = NamedTempFile::new().unwrap();

        let json = serde_json::json!({
            "task": {},
            "current": {
                "content_markdown": [{
                    "content": { "Delta": {} },
                    "page": 1
                }],
                "links": []
            },
            "historical_snapshots": [],
            "all_links": []
        });

        fs::write(file.path(), json.to_string()).unwrap();

        let result = current_content::<HistoricalPage>(file.path());
        assert!(result.is_none());
    }

    #[test]
    fn current_content_sorts_and_concatenates_pages() {
        let file = NamedTempFile::new().unwrap();

        let text1 = "a".repeat(6000);
        let text2 = "b".repeat(6000);

        let json = serde_json::json!({
                "task": {
                    "url_id": 0,
                    "url": "example.com",
                    "depth": 0,
                    "priority": 0
                },
            "current": {
                "content_markdown": [
                    {
                        "content": { "Literal": text2 },
                        "page": 2
                    },
                    {
                        "content": { "Literal": text1 },
                        "page": 1
                    }
                ],
                "links": []
            },
            "historical_snapshots": [],
            "all_links": []
        });

        fs::write(file.path(), json.to_string()).unwrap();

        let result = current_content::<HistoricalPage>(file.path()).unwrap();

        assert!(result.starts_with(&"a"));
        assert!(result.contains(&"b"));
    }

    #[test]
    fn current_content_returns_none_when_no_current() {
        let file = NamedTempFile::new().unwrap();

        let json = serde_json::json!({
            "task": {},
            "current": null,
            "historical_snapshots": [],
            "all_links": []
        });

        fs::write(file.path(), json.to_string()).unwrap();

        let result = current_content::<HistoricalPage>(file.path());
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn populate_vector_db_calls_embedder() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.json");

        let big_text = "a".repeat(6000);

        let json = serde_json::json!({
                "task": {
                    "url_id": 0,
                    "url": "example.com",
                    "depth": 0,
                    "priority": 0
                },
            "current": {
                "content_markdown": [{
                    "content": { "Literal": big_text },
                    "page": 1
                }],
                "links": []
            },
            "historical_snapshots": [],
            "all_links": []
        });

        fs::write(&file_path, json.to_string()).unwrap();

        let mut db = MockVectorDb::new();
        let mut embedder = MockEmbedder::new();

        embedder
            .expect_embed()
            .times(1)
            .returning(|chunks: Vec<String>, _| Ok(vec![vec![0.0; 3]; chunks.len()]));

        db.expect_upsert_points()
            .times(1)
            .returning(|_: UpsertPointsBuilder| {
                Ok(PointsOperationResponse {
                    result: None,
                    time: 0_f64,
                    usage: None,
                })
            });

        populate_vector_db(&mut embedder, &db, "test", dir.path().to_str().unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn populate_vector_db_calls_upsert() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.json");

        let big_text = "a".repeat(6000);

        let json = serde_json::json!({
                "task": {
                    "url_id": 0,
                    "url": "example.com",
                    "depth": 0,
                    "priority": 0
                },
            "current": {
                "content_markdown": [{
                    "content": { "Literal": big_text },
                    "page": 1
                }],
                "links": []
            },
            "historical_snapshots": [],
            "all_links": []
        });

        fs::write(&file_path, json.to_string()).unwrap();

        let mut db = MockVectorDb::new();
        let mut embedder = MockEmbedder::new();

        embedder
            .expect_embed()
            .returning(|chunks: Vec<String>, _| Ok(vec![vec![0.0; 3]; chunks.len()]));

        db.expect_upsert_points()
            .times(1)
            .returning(|_: UpsertPointsBuilder| {
                Ok(PointsOperationResponse {
                    result: None,
                    time: 0_f64,
                    usage: None,
                })
            });

        populate_vector_db(&mut embedder, &db, "test", dir.path().to_str().unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn populate_vector_db_handles_upsert_error() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.json");

        let big_text = "a".repeat(6000);

        let json = serde_json::json!({
            "task": {},
            "current": {
                "content_markdown": [{
                    "content": { "Literal": big_text },
                    "page": 1
                }],
                "links": []
            },
            "historical_snapshots": [],
            "all_links": []
        });

        fs::write(&file_path, json.to_string()).unwrap();

        let mut db = MockVectorDb::new();
        let mut embedder = MockEmbedder::new();

        embedder
            .expect_embed()
            .returning(|chunks: Vec<String>, _| Ok(vec![vec![0.0; 3]; chunks.len()]));

        db.expect_upsert_points()
            .returning(|_: UpsertPointsBuilder| {
                Err(qdrant_client::QdrantError::ConversionError(
                    "Something bad".into(),
                ))
            });

        let result =
            populate_vector_db(&mut embedder, &db, "test", dir.path().to_str().unwrap()).await;

        assert!(result.is_ok()); // should NOT fail overall
    }
}
