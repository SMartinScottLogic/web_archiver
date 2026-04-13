use std::collections::HashSet;

use anyhow::Result;
use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
use map_macro::hash_map;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tracing::{debug, info};

use common::{
    Archiver,
    historical::HistoricalPage,
    types::{ExtractedPage, FetchTask, PageMetadata},
};

pub mod weird;

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
pub struct LegacyStory {
    #[serde(alias = "_id")]
    pub id: String,
    pub story_id: String,
    pub page: usize,
    pub uri: String,
    pub story: Vec<String>,
    pub keywords: Vec<String>,
    pub title: String,
    pub author: String,
    #[serde(skip)]
    pub links: HashSet<String>,
}

pub fn parse_unambiguous_date(s: &str) -> Result<u64, String> {
    let formats = ["%Y-%m-%d", "%d/%m/%Y", "%m/%d/%Y", "%Y-%m-%d %H:%M:%S"];

    let mut results = Vec::new();

    for fmt in formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            results.push(Utc.from_utc_datetime(&dt).timestamp());
            continue;
        }

        if let Ok(date) = NaiveDate::parse_from_str(s, fmt)
            && let Some(dt) = date.and_hms_opt(0, 0, 0)
        {
            results.push(Utc.from_utc_datetime(&dt).timestamp());
        }
    }

    results.sort();
    results.dedup();

    match results.len() {
        0 => Err(format!(
            "Invalid date '{}'. Use YYYY-MM-DD (e.g. 2026-03-13)",
            s
        )),
        1 => Ok(results[0] as u64),
        _ => Err(format!("Ambiguous date '{}'. Use YYYY-MM-DD", s)),
    }
}

fn convert(path: &Path, fetch_time: u64) -> Result<ExtractedPage, anyhow::Error> {
    debug!(?path, "convert file");
    let text = fs::read_to_string(path)?;
    let content: LegacyStory = serde_json::from_str(&text)?;
    let markdown = content.story.join("\n\n");
    let url = content.uri;

    let url_id = 0;
    let discovered_from = None;

    let page = ExtractedPage {
        task: FetchTask {
                article_id: 0,
            url_id,
            url,
            depth: u32::MAX,
            priority: 0,
            discovered_from,
        },
        content_markdown: Some(markdown),
        links: content.links.into_iter().collect(),
        metadata: Some(PageMetadata {
            status_code: 200,
            content_type: None,
            fetch_time,
            title: Some(content.title),
            document_metadata: Some(vec![
                hash_map! {"keywords".to_string() => content.keywords.join(",")},
            ]),
        }),
    };
    Ok(page)
}

pub fn store_file(
    archiver: &impl Archiver,
    path: &Path,
    fetch_time: u64,
    delete_source: bool,
) -> Result<()> {
    let converted: HistoricalPage = convert(path, fetch_time)
        .or_else(|_| weird::read_file(path, fetch_time))?
        .into();
    // Save new file
    let destination = archiver.store_page(&converted)?;
    info!(source = ?path, ?destination, "page stored");
    if delete_source {
        fs::remove_file(path)?;
        debug!(?path, "Removed source file")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    use common::MockArchiver;
    use mockall::predicate::*;

    // ----------------------------
    // Helpers
    // ----------------------------

    fn tmp_file(name: &str, content: &str) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        fs::write(&path, content).unwrap();
        path
    }

    fn sample_legacy_json() -> String {
        serde_json::json!({
            "_id": "1",
            "story_id": "story1",
            "page": 1,
            "uri": "https://example.com",
            "story": ["para1", "para2"],
            "keywords": ["rust", "parser"],
            "title": "Test Title",
            "author": "Test Author"
        })
        .to_string()
    }

    // ----------------------------
    // parse_unambiguous_date tests
    // ----------------------------

    #[test]
    fn test_parse_unambiguous_date_valid_formats() {
        assert!(parse_unambiguous_date("2026-03-13").is_ok());
        assert!(parse_unambiguous_date("13/03/2026").is_ok());
        assert!(parse_unambiguous_date("03/13/2026").is_ok());
        assert!(parse_unambiguous_date("01/02/2026").is_err());
        assert!(parse_unambiguous_date("2026-03-13 12:00:00").is_ok());
    }

    #[test]
    fn test_parse_unambiguous_date_ambiguous() {
        let err = parse_unambiguous_date("01/02/2026").unwrap_err();
        assert!(err.contains("Ambiguous"));
    }

    #[test]
    fn test_parse_unambiguous_date_invalid() {
        let err = parse_unambiguous_date("not-a-date").unwrap_err();
        assert!(err.contains("Invalid"));
    }

    #[test]
    fn test_parse_unambiguous_date_consistent_timestamp() {
        let ts1 = parse_unambiguous_date("2026-03-13").unwrap();
        let ts2 = parse_unambiguous_date("2026-03-13 00:00:00").unwrap();
        assert_eq!(ts1, ts2);
    }

    // ----------------------------
    // convert tests
    // ----------------------------

    #[test]
    fn test_convert_basic() {
        let path = tmp_file("convert_basic.json", &sample_legacy_json());

        let page = convert(&path, 12345).unwrap();

        assert_eq!(page.task.url, "https://example.com");
        assert_eq!(
            page.metadata.as_ref().unwrap().title.as_ref().unwrap(),
            "Test Title"
        );

        let content = page.content_markdown.unwrap();
        assert!(content.contains("para1"));
        assert!(content.contains("para2"));

        let keywords = &page.metadata.unwrap().document_metadata.unwrap()[0]["keywords"];
        assert_eq!(keywords, "rust,parser");

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_convert_invalid_json() {
        let path = tmp_file("convert_invalid.json", "not json");

        let result = convert(&path, 12345);
        assert!(result.is_err());

        fs::remove_file(path).unwrap();
    }

    // ----------------------------
    // store_file tests (with mock)
    // ----------------------------

    #[test]
    fn test_store_file_success_no_delete() {
        let path = tmp_file("store_success.json", &sample_legacy_json());

        let mut mock = MockArchiver::new();
        mock.expect_store_page()
            .times(1)
            .returning(|_| Ok::<PathBuf, anyhow::Error>(PathBuf::from("stored_path")));

        let result = store_file(&mock, &path, 1111, false);
        assert!(result.is_ok());

        // file should still exist
        assert!(path.exists());

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_store_file_success_with_delete() {
        let path = tmp_file("store_delete.json", &sample_legacy_json());

        let mut mock = MockArchiver::new();
        mock.expect_store_page()
            .times(1)
            .returning(|_| Ok::<PathBuf, anyhow::Error>(PathBuf::from("stored_path")));

        let result = store_file(&mock, &path, 1111, true);
        assert!(result.is_ok());

        // file should be deleted
        assert!(!path.exists());
    }

    #[test]
    fn test_store_file_fallback_to_weird() {
        fn sample_input() -> String {
            r#"
        
My Story Title

by Jane Doe

Story URL: https://example.com/story
Packaged: 2024-01-01
TAGS: Rust Parsing
EXTRA URL: https://example.com/extra

This is the story content.
Second line.
"#
            .to_string()
        }
        // invalid JSON forces convert() to fail
        let path = tmp_file("fallback.txt", &sample_input());

        let mut mock = MockArchiver::new();
        mock.expect_store_page()
            .times(1)
            .returning(|_| Ok::<PathBuf, anyhow::Error>(PathBuf::from("stored_path")));

        // NOTE: this assumes weird::read_file succeeds for this input.
        // If it doesn't, you may want to mock weird::read_file separately.
        let _ = store_file(&mock, &path, 1111, false);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_store_file_archiver_error_propagates() {
        let path = tmp_file("archiver_error.json", &sample_legacy_json());

        let mut mock = MockArchiver::new();
        mock.expect_store_page()
            .times(1)
            .returning(|_| Err::<PathBuf, anyhow::Error>(anyhow::anyhow!("fail")));

        let result = store_file(&mock, &path, 1111, false);
        assert!(result.is_err());

        fs::remove_file(path).unwrap();
    }
}
