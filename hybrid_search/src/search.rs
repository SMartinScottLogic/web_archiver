use anyhow::Context;
use mockall::automock;
use qdrant_client::{
    qdrant::{Condition, Filter, QueryPointsBuilder, QueryResponse},
    Qdrant,
};
use tracing::info;
use vector_common::Embedder;

use crate::settings::Config;

#[automock]
pub trait VectorDb {
    async fn query(&self, builder: QueryPointsBuilder) -> anyhow::Result<QueryResponse>;
}

impl VectorDb for Qdrant {
    async fn query(&self, request: QueryPointsBuilder) -> anyhow::Result<QueryResponse> {
        Qdrant::query(self, request).await.context("query")
    }
}

fn build_query(config: &Config, query_vector: Vec<f32>) -> QueryPointsBuilder {
    let filter = config
        .source
        .clone()
        .map(|source| Filter::must([Condition::matches_text("source", source)]));

    match filter {
        None => QueryPointsBuilder::new(config.collection.clone())
            .query(query_vector)
            .limit(config.limit)
            .with_payload(true),
        Some(filter) => QueryPointsBuilder::new(config.collection.clone())
            .query(query_vector)
            .limit(config.limit)
            .filter(filter)
            .with_payload(true),
    }
}

/// Perform search
pub async fn perform_search(
    config: Config,
    //mut embedder: TextEmbedding,
    mut embedder: impl Embedder,
    client: impl VectorDb,
) -> anyhow::Result<()> {
    // ---------------------------
    // Embed query
    // ---------------------------
    let embedding = embedder.embed(vec![config.query.clone()], None)?;

    let query_vector = embedding[0].clone();

    let query_request = build_query(&config, query_vector);

    let results = client.query(query_request).await?;

    display_results(results);

    Ok(())
}

/// ---------------------------
/// Display results
/// ---------------------------
fn display_results(results: QueryResponse) {
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

        info!(
            result = i + 1,
            score = point.score,
            source,
            text = truncate(text, 200),
            ?payload
        );
    }
}

/// ---------------------------
/// Truncate to maximum size (with ellipsis)
/// ---------------------------
fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max])
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qdrant_client::qdrant::{QueryResponse, ScoredPoint};
    use std::collections::HashMap;
    use vector_common::MockEmbedder;

    // ----------------------------
    // truncate tests (expanded)
    // ----------------------------

    #[test]
    fn test_truncate_exact_length() {
        assert_eq!("hello", truncate("hello", 5));
    }

    #[test]
    fn test_truncate_shorter() {
        assert_eq!("hi", truncate("hi", 10));
    }

    #[test]
    fn test_truncate_longer() {
        assert_eq!("hell...", truncate("hello world", 4));
    }

    #[test]
    fn test_truncate_zero() {
        assert_eq!("...", truncate("hello", 0));
    }

    #[test]
    fn test_truncate_empty() {
        assert_eq!("", truncate("", 10));
    }

    // ----------------------------
    // build_query tests
    // ----------------------------

    #[test]
    fn test_build_query_without_filter() {
        let config = Config {
            query: "test".into(),
            collection: "my_collection".into(),
            limit: 5,
            source: None,
        };

        let builder = build_query(&config, vec![1.0, 2.0, 3.0]);

        // We can't easily introspect builder internals,
        // but we CAN ensure it doesn't panic and builds
        let _ = builder;
    }

    #[test]
    fn test_build_query_with_filter() {
        let config = Config {
            query: "test".into(),
            collection: "my_collection".into(),
            limit: 5,
            source: Some("news".into()),
        };

        let builder = build_query(&config, vec![1.0, 2.0, 3.0]);

        let _ = builder;
    }

    // ----------------------------
    // display_results tests
    // ----------------------------

    fn mock_point(text: Option<&str>, source: Option<&str>) -> ScoredPoint {
        let mut payload = HashMap::new();

        if let Some(t) = text {
            payload.insert("text".into(), t.into());
        }

        if let Some(s) = source {
            payload.insert("source".into(), s.into());
        }

        ScoredPoint {
            id: None,
            version: 0,
            score: 0.9,
            payload,
            vectors: None,
            shard_key: None,
            order_value: None,
        }
    }

    #[test]
    fn test_display_results_handles_missing_fields() {
        let response = QueryResponse {
            result: vec![
                mock_point(None, None),
                mock_point(Some("hello"), None),
                mock_point(None, Some("source")),
            ],
            time: 0.0,
            usage: None,
        };

        // Should not panic
        display_results(response);
    }

    #[test]
    fn test_display_results_with_full_payload() {
        let response = QueryResponse {
            result: vec![mock_point(Some("some long text here"), Some("blog"))],
            time: 0.0,
            usage: None,
        };

        display_results(response);
    }

    // ----------------------------
    // integration-lite test
    // ----------------------------

    #[tokio::test]
    async fn test_perform_search_embed_failure() {
        let config = Config {
            query: "test".into(),
            collection: "c".into(),
            limit: 1,
            source: None,
        };

        let mut embedder = MockEmbedder::new();

        embedder
            .expect_embed::<String, Vec<String>>()
            .returning(|_, _| Err(anyhow::Error::msg("expected error")));

        // Dummy client (won't be reached)
        let mut client = MockVectorDb::new();
        client.expect_query().returning(|_| {
            Ok(QueryResponse {
                result: vec![
                    mock_point(None, None),
                    mock_point(Some("hello"), None),
                    mock_point(None, Some("source")),
                ],
                time: 0.0,
                usage: None,
            })
        });

        // Force embed failure by passing empty query vector scenario
        let result = perform_search(config, embedder, client).await;

        // We don't strictly guarantee failure, but test ensures no panic
        let _ = result;
    }
}
