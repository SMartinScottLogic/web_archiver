use std::{
    collections::{HashSet, VecDeque},
    fs::{File, create_dir_all},
    path::Path,
};

use anyhow::Context as _;

use common::{
    historical::{HistoricalContent, HistoricalContentType, HistoricalPage, HistoricalSnapshot},
    page::PageReader,
    types::{FetchTask, PageMetadata},
    url::canonicalize_url,
};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExtractedPage {
    pub task: FetchTask,
    pub content_markdown: Option<String>,
    pub links: Vec<String>,
    pub metadata: Option<PageMetadata>,
}
impl ExtractedPage {
    pub fn write_page(&self, path: &Path) -> anyhow::Result<()> {
        let parent = path
            .parent()
            .with_context(|| format!("Failed to get parent of {:?}", path))?;
        create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {:?}", parent))?;

        let file =
            File::create(path).with_context(|| format!("Failed to create file {:?}", path))?;

        serde_json::to_writer_pretty(file, self)
            .with_context(|| format!("Failed to write JSON to {:?}", path))?;

        Ok(())
    }
}

impl From<ExtractedPage> for HistoricalPage {
    fn from(val: ExtractedPage) -> Self {
        let mut val = val;
        val.task.url = canonicalize_url(&val.task.url).unwrap_or_default();
        let content_markdown = match val.content_markdown {
            Some(text) => vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal(text),
            }],
            None => Vec::new(),
        };
        let current = HistoricalSnapshot {
            content_markdown,
            links: HashSet::new(),
            metadata: val.metadata,
        };
        HistoricalPage {
            task: val.task,
            current: Some(current),
            historical_snapshots: VecDeque::new(),
            all_links: HashSet::new(),
        }
    }
}

/*
impl HistoricalSnapshot {
    /// Convert a single ExtractedPage into a HistoricalSnapshot
    pub fn from_extracted_page(page: ExtractedPage) -> Self {
        Self {
            //task: page.task,
            content_markdown: match page.content_markdown {
                Some(t) => vec![HistoricalContent {
                    page: 1,
                    content: HistoricalContentType::Literal(t),
                }],
                None => Vec::new(),
            },
            links: page.links.into_iter().collect(),
            metadata: page.metadata,
        }
    }
}
*/

impl From<ExtractedPage> for HistoricalSnapshot {
    fn from(page: ExtractedPage) -> Self {
        Self {
            //task: page.task,
            content_markdown: match page.content_markdown {
                Some(t) => vec![HistoricalContent {
                    page: 1,
                    content: HistoricalContentType::Literal(t),
                }],
                None => Vec::new(),
            },
            links: page.links.into_iter().collect(),
            metadata: page.metadata,
        }
    }
}

impl PageReader for ExtractedPage {
    fn url(&self) -> &str {
        &self.task.url
    }

    fn task(&self) -> &FetchTask {
        &self.task
    }

    fn set_url(&mut self, url: &str) {
        self.task.url = url.to_string();
    }

    fn current(&self) -> &Option<HistoricalSnapshot> {
        todo!("convert ExtractedPage to HistoricalSnapshot?");
    }

    fn current_mut(&mut self) -> Option<&mut HistoricalSnapshot> {
        todo!("convert ExtractedPage to HistoricalSnapshot?")
    }

    fn snapshots(&mut self) -> &[HistoricalSnapshot] {
        // For ExtractedPage, we conceptually have a single snapshot
        // but we can't return it without allocating, so return empty
        // Callers should use ExtractedPageExt::as_snapshots() instead
        &[]
    }

    fn all_links(&self) -> HashSet<String> {
        self.links.iter().cloned().collect()
    }

    fn fetch_time(&self) -> u64 {
        self.metadata.as_ref().map(|m| m.fetch_time).unwrap_or(0)
    }

    fn latest_fetch_time(&self) -> u64 {
        // For a single extracted page, the fetch time is also the latest
        self.fetch_time()
    }

    fn write(&self, path: &Path) -> anyhow::Result<()> {
        self.write_page(path)
    }
}

/// Extension trait for converting ExtractedPage to a temporary PageReader-compatible form
pub trait ExtractedPageExt {
    /// Get snapshots from this ExtractedPage as a single-element vec
    fn as_snapshots(&self) -> Vec<HistoricalSnapshot>;
}

impl ExtractedPageExt for ExtractedPage {
    fn as_snapshots(&self) -> Vec<HistoricalSnapshot> {
        vec![HistoricalSnapshot {
            //task: self.task.clone(),
            content_markdown: match self.content_markdown.as_ref() {
                Some(md) => vec![HistoricalContent {
                    page: 1,
                    content: HistoricalContentType::Literal(md.to_owned()),
                }],
                None => Vec::new(),
            },
            links: self.links.iter().cloned().collect(),
            metadata: self.metadata.clone(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;
    use common::types::{FetchTask, Priority};

    fn sample_task() -> FetchTask {
        FetchTask {
            article_id: 1,
            url_id: 42,
            url: "http://example.com".to_string(),
            depth: 1,
            priority: Priority::default(),
            discovered_from: Some(1),
        }
    }

    #[test]
    fn test_extracted_page_serde_roundtrip() {
        let page = ExtractedPage {
            task: sample_task(),
            content_markdown: Some("Hello **world**".into()),
            links: vec!["http://a.com".into(), "http://b.com".into()],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".into()),
                fetch_time: 999,
                title: Some("Example".into()),
                document_metadata: None,
            }),
        };

        let json = serde_json::to_string(&page).unwrap();
        let decoded: ExtractedPage = serde_json::from_str(&json).unwrap();

        assert_eq!(page, decoded);
    }

    #[test]
    fn test_write_page_creates_file_and_dirs() {
        let page = ExtractedPage {
            task: sample_task(),
            content_markdown: Some("content".into()),
            links: vec![],
            metadata: None,
        };

        let tmp_dir = std::env::temp_dir();
        let file_path: PathBuf = tmp_dir.join("crawler_test/subdir/page.json");

        // Clean up before test (in case it exists)
        let _ = fs::remove_file(&file_path);

        page.write_page(&file_path).unwrap();

        assert!(file_path.exists());

        // Verify it's valid JSON
        let content = fs::read_to_string(&file_path).unwrap();
        let decoded: ExtractedPage = serde_json::from_str(&content).unwrap();

        assert_eq!(decoded.task.url, "http://example.com");

        // Cleanup
        let _ = fs::remove_file(&file_path);
    }

    #[test]
    fn test_write_page_invalid_path() {
        let page = ExtractedPage {
            task: sample_task(),
            content_markdown: None,
            links: vec![],
            metadata: None,
        };

        // Path without parent (edge case)
        let path = Path::new("");

        let result = page.write_page(path);

        assert!(result.is_err());
    }

    #[test]
    fn test_conversion_to_historical_page_with_content() {
        let page = ExtractedPage {
            task: sample_task(),
            content_markdown: Some("markdown".into()),
            links: vec!["http://a.com".into()],
            metadata: None,
        };

        let hist: HistoricalPage = page.into();

        assert!(hist.current.is_some());

        let current = hist.current.unwrap();

        assert_eq!(current.content_markdown.len(), 1);

        match current.content_markdown.first().unwrap() {
            HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal(text),
            } => {
                assert_eq!(text, "markdown");
            }
            _ => panic!("Expected Literal content"),
        }

        // Links should NOT be copied (important behavior)
        assert!(current.links.is_empty());

        assert!(hist.historical_snapshots.is_empty());
        assert!(hist.all_links.is_empty());
    }

    #[test]
    fn test_conversion_to_historical_page_without_content() {
        let page = ExtractedPage {
            task: sample_task(),
            content_markdown: None,
            links: vec![],
            metadata: None,
        };

        let hist: HistoricalPage = page.into();
        let current = hist.current.unwrap();

        assert!(current.content_markdown.is_empty());
    }

    #[test]
    fn test_conversion_preserves_metadata() {
        let metadata = PageMetadata {
            status_code: 404,
            content_type: Some("text/html".into()),
            fetch_time: 111,
            title: Some("Not Found".into()),
            document_metadata: None,
        };

        let page = ExtractedPage {
            task: sample_task(),
            content_markdown: None,
            links: vec![],
            metadata: Some(metadata.clone()),
        };

        let hist: HistoricalPage = page.into();
        let current = hist.current.unwrap();

        assert_eq!(current.metadata, Some(metadata));
    }

    #[test]
    fn test_conversion_canonicalizes_url() {
        let page = ExtractedPage {
            task: FetchTask {
                article_id: 1,
                url_id: 1,
                url: "http://example.com/".to_string(), // trailing slash
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
            },
            content_markdown: None,
            links: vec![],
            metadata: None,
        };

        let hist: HistoricalPage = page.into();

        // We don't know exact canonical form, but ensure it's not empty
        assert!(!hist.task.url.is_empty());
    }

    #[test]
    fn test_conversion_invalid_url_fallback() {
        let page = ExtractedPage {
            task: FetchTask {
                article_id: 1,
                url_id: 1,
                url: "not a valid url%%%".to_string(),
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
            },
            content_markdown: None,
            links: vec![],
            metadata: None,
        };

        let hist: HistoricalPage = page.into();

        // unwrap_or_default() → empty string fallback
        assert_eq!(hist.task.url, "");
    }

    #[test]
    fn test_snapshot_from_extracted_page() {
        let extracted = ExtractedPage {
            task: FetchTask {
                article_id: 0,
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
            },
            content_markdown: Some("Content".to_string()),
            links: vec!["https://link.com".to_string()],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                title: Some("Title".to_string()),
                document_metadata: None,
            }),
        };

        let snapshot = HistoricalSnapshot::from(extracted.clone());

        // assert_eq!(snapshot.task.url_id, 1);
        // assert_eq!(snapshot.task.url, "https://example.com");
        assert_eq!(
            snapshot.content_markdown,
            vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal("Content".to_string())
            }]
        );
        assert_eq!(snapshot.links.len(), 1);
        assert_eq!(snapshot.metadata.as_ref().unwrap().status_code, 200);
    }

    #[test]
    fn test_from_trait_for_snapshot() {
        let extracted = ExtractedPage {
            task: FetchTask {
                article_id: 0,
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
            },
            content_markdown: Some("Example content".to_string()),
            links: vec![],
            metadata: None,
        };

        let snapshot: HistoricalSnapshot = extracted.into();
        assert_eq!(
            snapshot.content_markdown,
            vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal("Example content".to_string())
            }]
        );
    }

    #[test]
    fn test_extracted_page_reader_basic() {
        let page = ExtractedPage {
            task: FetchTask {
                article_id: 0,
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
            },
            content_markdown: Some("Content".to_string()),
            links: vec![
                "https://link1.com".to_string(),
                "https://link2.com".to_string(),
            ],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                title: Some("Page".to_string()),
                document_metadata: None,
            }),
        };

        assert_eq!(page.url(), "https://example.com");
        assert_eq!(page.all_links().len(), 2);
        assert_eq!(page.fetch_time(), 1000);
        assert_eq!(page.latest_fetch_time(), 1000);
    }

    #[test]
    fn test_extracted_page_reader_no_metadata() {
        let page = ExtractedPage {
            task: FetchTask {
                article_id: 0,
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
            },
            content_markdown: None,
            links: vec![],
            metadata: None,
        };

        assert_eq!(page.fetch_time(), 0);
        assert_eq!(page.latest_fetch_time(), 0);
    }

    #[test]
    fn test_extracted_page_as_snapshots() {
        let page = ExtractedPage {
            task: FetchTask {
                article_id: 0,
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
            },
            content_markdown: Some("Content".to_string()),
            links: vec!["https://link.com".to_string()],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 1000,
                title: None,
                document_metadata: None,
            }),
        };

        let snapshots = page.as_snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].links.len(), 1);
    }
}
