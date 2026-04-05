use std::path::Path;

use mockall::automock;

use crate::historical::{HistoricalContentType, HistoricalPage, HistoricalSnapshot};
use crate::types::ExtractedPage;

/// A trait for reading page data, abstracting over both ExtractedPage and HistoricalPage.
/// This enables crates to work with either type without knowing which concrete implementation they have.
#[automock]
#[allow(clippy::needless_lifetimes)]
pub trait PageReader {
    /// Get the canonical URL for this page
    fn url(&self) -> &str;

    /// Set the canonical URL for this page
    fn set_url(&mut self, url: &str);

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
    fn all_links(&self) -> Vec<String>;

    /// Get the fetch time of the earliest/oldest snapshot
    fn fetch_time(&self) -> u64;

    /// Get the fetch time of the most recent snapshot
    fn latest_fetch_time(&self) -> u64;

    /// Write the page to the supplied path
    fn write(&self, path: &Path) -> anyhow::Result<()>;
}

impl PageReader for ExtractedPage {
    fn url(&self) -> &str {
        &self.task.url
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

    fn all_links(&self) -> Vec<String> {
        self.links.clone()
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

impl PageReader for HistoricalPage {
    fn url(&self) -> &str {
        &self.url
    }

    fn set_url(&mut self, url: &str) {
        self.url = url.to_string();
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

    fn all_links(&self) -> Vec<String> {
        // Return sorted links (consistent with serialization)
        let mut links: Vec<_> = self.all_links.iter().cloned().collect();
        links.sort();
        links
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

/// Extension trait for converting ExtractedPage to a temporary PageReader-compatible form
pub trait ExtractedPageExt {
    /// Get snapshots from this ExtractedPage as a single-element vec
    fn as_snapshots(&self) -> Vec<HistoricalSnapshot>;
}

impl ExtractedPageExt for ExtractedPage {
    fn as_snapshots(&self) -> Vec<HistoricalSnapshot> {
        vec![HistoricalSnapshot {
            task: self.task.clone(),
            content_markdown: match self.content_markdown.as_ref() {
                Some(md) => HistoricalContentType::Literal(md.to_owned()),
                None => HistoricalContentType::None,
            },
            links: self.links.clone(),
            metadata: self.metadata.clone(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FetchTask, PageMetadata};

    #[test]
    fn test_extracted_page_reader_basic() {
        let page = ExtractedPage {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
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
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
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
    fn test_historical_page_reader_basic() {
        let mut page = HistoricalPage::new("https://example.com".to_string());

        let snapshot = HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal("Content".to_string()),
            links: vec![
                "https://link1.com".to_string(),
                "https://link2.com".to_string(),
            ],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 1000,
                title: None,
                document_metadata: None,
            }),
        };

        page.add_snapshot(snapshot);

        assert_eq!(page.url(), "https://example.com");
        assert!(page.current.is_some());
        assert_eq!(page.snapshots().len(), 0);
        assert_eq!(page.all_links().len(), 2);
        assert_eq!(page.fetch_time(), 1000);
        assert_eq!(page.latest_fetch_time(), 1000);
    }

    #[test]
    fn test_historical_page_reader_multiple_snapshots() {
        let mut page = HistoricalPage::new("https://example.com".to_string());

        let snapshot1 = HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal("content version 1".to_string()),
            links: vec!["https://link1.com".to_string()],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 1000,
                title: None,
                document_metadata: None,
            }),
        };

        let snapshot2 = HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal("content version 2".to_string()),
            links: vec!["https://link2.com".to_string()],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 2000,
                title: None,
                document_metadata: None,
            }),
        };

        page.add_snapshot(snapshot1);
        page.add_snapshot(snapshot2);

        assert!(page.current.is_some());
        assert_eq!(page.snapshots().len(), 1);
        assert_eq!(page.fetch_time(), 1000); // oldest
        assert_eq!(page.latest_fetch_time(), 2000); // newest
        assert_eq!(page.all_links().len(), 2);
    }

    #[test]
    fn test_extracted_page_as_snapshots() {
        let page = ExtractedPage {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
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
