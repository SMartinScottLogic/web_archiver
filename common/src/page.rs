use crate::historical::{HistoricalPage, HistoricalSnapshot};
use crate::types::ExtractedPage;

/// A trait for reading page data, abstracting over both ExtractedPage and HistoricalPage.
/// This enables crates to work with either type without knowing which concrete implementation they have.
pub trait PageReader {
    /// Get the canonical URL for this page
    fn url(&self) -> &str;

    /// Get all historical snapshots for this page
    /// For ExtractedPage: returns a single-element slice
    /// For HistoricalPage: returns all snapshots
    fn snapshots(&self) -> &[HistoricalSnapshot];

    /// Get all unique links discovered across all snapshots
    /// For ExtractedPage: returns links from the single snapshot
    /// For HistoricalPage: returns the consolidated deduplicated links (as sorted Vec)
    fn all_links(&self) -> Vec<String>;

    /// Get the fetch time of the earliest/oldest snapshot
    fn fetch_time(&self) -> u64;

    /// Get the fetch time of the most recent snapshot
    fn latest_fetch_time(&self) -> u64;
}

impl PageReader for ExtractedPage {
    fn url(&self) -> &str {
        &self.task.url
    }

    fn snapshots(&self) -> &[HistoricalSnapshot] {
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
}

impl PageReader for HistoricalPage {
    fn url(&self) -> &str {
        &self.url
    }

    fn snapshots(&self) -> &[HistoricalSnapshot] {
        &self.historical_snapshots
    }

    fn all_links(&self) -> Vec<String> {
        // Return sorted links (consistent with serialization)
        let mut links: Vec<_> = self.all_links.iter().cloned().collect();
        links.sort();
        links
    }

    fn fetch_time(&self) -> u64 {
        self.historical_snapshots
            .first()
            .and_then(|s| s.metadata.as_ref().map(|m| m.fetch_time))
            .unwrap_or(0)
    }

    fn latest_fetch_time(&self) -> u64 {
        self.historical_snapshots
            .last()
            .and_then(|s| s.metadata.as_ref().map(|m| m.fetch_time))
            .unwrap_or(0)
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
            content_markdown: self.content_markdown.clone(),
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
            content_markdown: Some("Content".to_string()),
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
        assert_eq!(page.snapshots().len(), 1);
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
            content_markdown: None,
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
            content_markdown: None,
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

        assert_eq!(page.snapshots().len(), 2);
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
