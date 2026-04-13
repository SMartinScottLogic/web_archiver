use std::collections::HashMap;

use common::types::ExtractedPage;
use common::url::extract_domain;

use common::url::{extract_page, normalize_url_for_merge};

/// A key for grouping pages by (domain, normalized_url)
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AggregateKey {
    pub domain: String,
    pub normalized_url: String,
}

/// A page entry with optional page number for multi-page merging
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct PageEntry {
    pub page: ExtractedPage,
    pub page_number: Option<u32>,
}

/// Aggregates ExtractedPages by (domain, normalized_url) for multi-page merging.
///
/// Groups pages with the same normalized URL so that multi-page articles
/// (identified by ?page=X parameters) can be merged into a single snapshot.
pub struct ArchiveAggregator {
    /// HashMap keyed by (domain, normalized_url)
    /// Value is Vec of pages grouped by (url, page_number)
    aggregates: HashMap<AggregateKey, Vec<PageEntry>>,
}

impl ArchiveAggregator {
    /// Create a new empty aggregator
    pub fn new() -> Self {
        Self {
            aggregates: HashMap::new(),
        }
    }

    /// Add a page to the aggregator.
    /// Groups it by (domain, normalized_url) and tracks page number if present.
    ///
    /// Returns true if the page was successfully added, false if URL parsing failed.
    pub fn add_page(&mut self, page: ExtractedPage) -> bool {
        // Extract domain from the URL
        let domain = match extract_domain(&page.task.url) {
            Some(d) => d,
            None => {
                return false;
            }
        };

        // Normalize the URL (remove pagination params)
        let normalized = match normalize_url_for_merge(&page.task.url) {
            Some(n) => n,
            None => {
                return false;
            }
        };

        // Extract page number if present
        let page_number = match extract_page(&page.task.url) {
            common::url::Page::Number(page_number) => Some(page_number),
            common::url::Page::Text(_) => None,
            common::url::Page::None => None,
        };

        let key = AggregateKey {
            domain,
            normalized_url: normalized,
        };

        // Insert into the aggregates map
        self.aggregates
            .entry(key)
            .or_default()
            .push(PageEntry { page, page_number });

        true
    }

    /// Get the aggregated pages, consuming the aggregator
    #[allow(dead_code)]
    pub fn into_aggregates(self) -> HashMap<AggregateKey, Vec<PageEntry>> {
        self.aggregates
    }

    /// Get a reference to the aggregates
    #[allow(dead_code)]
    pub fn aggregates(&self) -> &HashMap<AggregateKey, Vec<PageEntry>> {
        &self.aggregates
    }

    /// Get the number of unique URLs
    #[allow(dead_code)]
    pub fn unique_urls(&self) -> usize {
        self.aggregates.len()
    }

    /// Get the total number of pages across all aggregates
    #[allow(dead_code)]
    pub fn total_pages(&self) -> usize {
        self.aggregates.values().map(|v| v.len()).sum()
    }
}

impl Default for ArchiveAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::types::{FetchTask, PageMetadata};

    fn make_page(url: &str, fetch_time: u64) -> ExtractedPage {
        ExtractedPage {
            task: FetchTask {
                article_id: 0,
                url_id: 1,
                url: url.to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: Some("content".to_string()),
            links: vec![],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time,
                title: Some("Title".to_string()),
                document_metadata: None,
            }),
        }
    }

    #[test]
    fn test_aggregator_new() {
        let agg = ArchiveAggregator::new();
        assert_eq!(agg.unique_urls(), 0);
        assert_eq!(agg.total_pages(), 0);
    }

    #[test]
    fn test_aggregator_add_single_page() {
        let mut agg = ArchiveAggregator::new();
        let page = make_page("http://example.com/article", 100);

        assert!(agg.add_page(page));
        assert_eq!(agg.unique_urls(), 1);
        assert_eq!(agg.total_pages(), 1);
    }

    #[test]
    fn test_aggregator_groups_multi_page() {
        let mut agg = ArchiveAggregator::new();
        let page1 = make_page("http://example.com/article?page=1", 100);
        let page2 = make_page("http://example.com/article?page=2", 100);

        agg.add_page(page1);
        agg.add_page(page2);

        assert_eq!(
            agg.unique_urls(),
            1,
            "Multi-page articles should consolidate to single URL"
        );
        assert_eq!(agg.total_pages(), 2);
    }

    #[test]
    fn test_aggregator_different_urls_separate() {
        let mut agg = ArchiveAggregator::new();
        let page1 = make_page("http://example.com/article1", 100);
        let page2 = make_page("http://example.com/article2", 100);

        agg.add_page(page1);
        agg.add_page(page2);

        assert_eq!(agg.unique_urls(), 2);
        assert_eq!(agg.total_pages(), 2);
    }

    #[test]
    fn test_aggregator_extract_page_number() {
        let mut agg = ArchiveAggregator::new();
        let page = make_page("http://example.com/article?page=5", 100);

        agg.add_page(page);

        let aggregates = agg.into_aggregates();
        let entries = aggregates.values().next().unwrap();
        assert_eq!(entries[0].page_number, Some(5));
    }

    #[test]
    fn test_aggregator_different_domains_separate() {
        let mut agg = ArchiveAggregator::new();
        let page1 = make_page("http://example.com/article", 100);
        let page2 = make_page("http://other.com/article", 100);

        agg.add_page(page1);
        agg.add_page(page2);

        assert_eq!(agg.unique_urls(), 2);
    }

    #[test]
    fn test_aggregator_invalid_url() {
        let mut agg = ArchiveAggregator::new();
        let mut page = make_page("http://example.com/article", 100);
        page.task.url = "not a url".to_string();

        assert!(!agg.add_page(page));
        assert_eq!(agg.unique_urls(), 0);
    }
}
