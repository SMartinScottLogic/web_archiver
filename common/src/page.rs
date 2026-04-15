use std::collections::HashSet;
use std::path::Path;

use mockall::automock;

use crate::historical::{HistoricalPage, HistoricalSnapshot};
use crate::types::FetchTask;

/// A trait for reading page data, abstracting over both ExtractedPage and HistoricalPage.
/// This enables crates to work with either type without knowing which concrete implementation they have.
#[automock]
#[allow(clippy::needless_lifetimes)]
pub trait PageReader {
    /// Get the canonical URL for this page
    fn url(&self) -> &str;
    /// Get the fetch details for this page
    fn task(&self) -> &FetchTask;

    /// Get current snapshot for this page
    fn current(&self) -> &Option<HistoricalSnapshot>;

    /// Get mutable reference to the current snapshot for this page
    fn current_mut<'a>(&'a mut self) -> Option<&'a mut HistoricalSnapshot>;

    /// Get historical snapshots (NOT current) for this page
    /// For ExtractedPage: returns an empty slice
    /// For HistoricalPage: returns historical snapshots
    fn snapshots(&mut self) -> &[HistoricalSnapshot];

    /// Get all unique links discovered across all snapshots
    /// For ExtractedPage: returns links from the single snapshot
    /// For HistoricalPage: returns the consolidated deduplicated links (as sorted Vec)
    fn all_links(&self) -> HashSet<String>;

    /// Get the fetch time of the earliest/oldest snapshot
    fn fetch_time(&self) -> u64;

    /// Get the fetch time of the most recent snapshot
    fn latest_fetch_time(&self) -> u64;

    /// Write the page to the supplied path
    fn write(&self, path: &Path) -> anyhow::Result<()>;
}

impl PageReader for HistoricalPage {
    fn url(&self) -> &str {
        &self.task.url
    }

    fn task(&self) -> &FetchTask {
        &self.task
    }

    fn current(&self) -> &Option<HistoricalSnapshot> {
        &self.current
    }

    fn current_mut(&mut self) -> Option<&mut HistoricalSnapshot> {
        self.current.as_mut()
    }

    fn snapshots(&mut self) -> &[HistoricalSnapshot] {
        self.historical_snapshots.make_contiguous()
    }

    fn all_links(&self) -> HashSet<String> {
        // Return sorted links (consistent with serialization)
        self.all_links.clone()
    }

    fn fetch_time(&self) -> u64 {
        if self.historical_snapshots.is_empty() {
            self.latest_fetch_time()
        } else {
            self.historical_snapshots
                .iter()
                .next_back()
                .and_then(|s| s.metadata.as_ref().map(|m| m.fetch_time))
                .unwrap_or(0)
        }
    }

    fn latest_fetch_time(&self) -> u64 {
        self.current
            .as_ref()
            .and_then(|s| s.metadata.as_ref().map(|m| m.fetch_time))
            .unwrap_or(0)
    }

    fn write(&self, path: &Path) -> anyhow::Result<()> {
        self.write_page(path)
    }
}

#[cfg(test)]
mod tests {
    use tracing_test::traced_test;

    use super::*;
    use crate::{
        historical::{HistoricalContent, HistoricalContentType},
        types::{FetchTask, PageMetadata, Priority},
    };

    #[test]
    fn test_historical_page_reader_basic() {
        let mut page = HistoricalPage::new(FetchTask {
            article_id: 0,
            url_id: 0,
            url: "https://example.com".to_string(),
            depth: 0,
            priority: Priority::default(),
            discovered_from: None,
        });

        let snapshot = HistoricalSnapshot {
            // task: FetchTask {
            //     url_id: 1,
            //     url: "https://example.com".to_string(),
            //     depth: 0,
            //     priority: 0,
            //     discovered_from: None,
            // },
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal("Content".to_string()),
            }],
            links: HashSet::from([
                "https://link1.com".to_string(),
                "https://link2.com".to_string(),
            ]),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 1000,
                title: None,
                document_metadata: None,
            }),
        };

        page.add_snapshot(snapshot).unwrap();

        assert_eq!(page.url(), "https://example.com");
        assert!(page.current.is_some());
        assert_eq!(page.snapshots().len(), 0);
        assert_eq!(page.all_links().len(), 2);
        assert_eq!(page.fetch_time(), 1000);
        assert_eq!(page.latest_fetch_time(), 1000);
    }

    #[test]
    #[traced_test]
    fn test_historical_page_reader_multiple_snapshots() {
        let mut page = HistoricalPage::new(FetchTask {
            article_id: 0,
            url_id: 0,
            url: "https://example.com".to_string(),
            depth: 0,
            priority: Priority::default(),
            discovered_from: None,
        });

        let snapshot1 = HistoricalSnapshot {
            // task: FetchTask {
            //     url_id: 1,
            //     url: "https://example.com".to_string(),
            //     depth: 0,
            //     priority: 0,
            //     discovered_from: None,
            // },
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal("content version 1".to_string()),
            }],
            links: HashSet::from(["https://link1.com".to_string()]),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 1000,
                title: None,
                document_metadata: None,
            }),
        };

        let snapshot2 = HistoricalSnapshot {
            // task: FetchTask {
            //     url_id: 1,
            //     url: "https://example.com".to_string(),
            //     depth: 0,
            //     priority: 0,
            //     discovered_from: None,
            // },
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal(
                    "content version Two, with enhanced details".to_string(),
                ),
            }],
            links: HashSet::from(["https://link2.com".to_string()]),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 2000,
                title: None,
                document_metadata: None,
            }),
        };

        page.add_snapshot(snapshot1).unwrap();
        page.add_snapshot(snapshot2).unwrap();

        assert!(page.current.is_some());
        assert_eq!(page.snapshots().len(), 1);
        assert_eq!(page.fetch_time(), 1000); // oldest
        assert_eq!(page.latest_fetch_time(), 2000); // newest
        assert_eq!(page.all_links().len(), 2);
    }
}
