use std::collections::HashMap;

use common::types::ExtractedPage;

use crate::{aggregator::PageEntry, historical_serializer::is_leap_year};

/// Extract year and month from Unix timestamp
/// Returns (year, month) where month is 1-12
fn fetch_time_to_year_month(fetch_time: u64) -> (u32, u32) {
    // Convert Unix timestamp to days since epoch
    let days_since_epoch = fetch_time / 86400; // seconds per day

    let mut days = days_since_epoch;
    let mut current_year = 1970u32;

    // Skip to the correct year
    while current_year < 2100 {
        let days_in_year = if is_leap_year(current_year) { 366 } else { 365 };
        if days < days_in_year as u64 {
            break;
        }
        days -= days_in_year as u64;
        current_year += 1;
    }

    // Now find the month
    let is_leap = is_leap_year(current_year);
    let days_in_months = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u32;
    for (i, &days_in_month) in days_in_months.iter().enumerate() {
        if days < days_in_month as u64 {
            month = (i + 1) as u32;
            break;
        }
        days -= days_in_month as u64;
    }

    (current_year, month)
}

/// A snapshot result after merging multi-page articles.
/// Contains the merged content and combined links.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct MergedSnapshot {
    /// The base page (first page or lowest page number)
    pub base_page: ExtractedPage,
    /// Merged markdown content from all pages in order
    pub merged_content: String,
    /// All links collected from all pages (order preserved from merging)
    pub merged_links: Vec<String>,
    /// Number of pages merged (1 if no merging occurred)
    pub page_count: usize,
}

/// Merges multiple pages with the same URL and fetch month into a single snapshot.
///
/// Handles:
/// - Grouping pages by year-month (not exact timestamp)
/// - Sorting pages by page number (if present)
/// - Concatenating markdown with clear separators
/// - Combining links while preserving order
/// - Selecting metadata from the base page
pub fn merge_pages_by_date(pages: &[PageEntry]) -> HashMap<(u32, u32), MergedSnapshot> {
    // Group pages by year-month (not exact fetch_time)
    let mut by_year_month: HashMap<(u32, u32), Vec<&PageEntry>> = HashMap::new();

    for entry in pages {
        let fetch_time = entry
            .page
            .metadata
            .as_ref()
            .map(|m| m.fetch_time)
            .unwrap_or(0);

        let year_month = fetch_time_to_year_month(fetch_time);

        by_year_month.entry(year_month).or_default().push(entry);
    }

    // Merge each group and return results
    let mut results = HashMap::new();
    for (year_month, mut entries) in by_year_month {
        // Sort by page number (ascending), then by URL (for stable ordering)
        entries.sort_by(|a, b| {
            match (a.page_number, b.page_number) {
                (Some(pa), Some(pb)) => pa.cmp(&pb),
                (Some(_), None) => std::cmp::Ordering::Less, // Pages with numbers come first
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.page.task.url.cmp(&b.page.task.url),
            }
        });

        // Merge the content and links
        let base_page = entries[0].page.clone();
        let mut merged_content = String::new();
        let mut merged_links = Vec::new();
        let mut seen_links = std::collections::HashSet::new();

        for (i, entry) in entries.iter().enumerate() {
            // Add page content with separator for multi-page
            if i > 0 {
                merged_content.push_str("\n\n---\n\n");
                if let Some(page_num) = entry.page_number {
                    merged_content.push_str(&format!("## Page {}\n\n", page_num));
                }
            }

            if let Some(content) = &entry.page.content_markdown {
                merged_content.push_str(content);
            }

            // Collect unique links
            for link in &entry.page.links {
                if seen_links.insert(link.clone()) {
                    merged_links.push(link.clone());
                }
            }
        }

        let merged = MergedSnapshot {
            base_page,
            merged_content,
            merged_links,
            page_count: entries.len(),
        };

        results.insert(year_month, merged);
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::types::{FetchTask, PageMetadata};

    fn make_page(
        url: &str,
        page_num: Option<u32>,
        fetch_time: u64,
        content: &str,
        links: Vec<String>,
    ) -> PageEntry {
        PageEntry {
            page: ExtractedPage {
                task: FetchTask {
                    url_id: 1,
                    url: url.to_string(),
                    depth: 0,
                    priority: 0,
                    discovered_from: None,
                },
                content_markdown: Some(content.to_string()),
                links,
                metadata: Some(PageMetadata {
                    status_code: 200,
                    content_type: Some("text/html".to_string()),
                    fetch_time,
                    title: Some("Title".to_string()),
                    document_metadata: None,
                }),
            },
            page_number: page_num,
        }
    }

    #[test]
    fn test_merge_single_page() {
        let pages = vec![make_page(
            "http://example.com/article",
            None,
            100,
            "Page content",
            vec!["http://link1.com".to_string()],
        )];

        let result = merge_pages_by_date(&pages);
        assert_eq!(result.len(), 1);

        // fetch_time=100 is 1970-01-01, so year_month should be (1970, 1)
        let snapshot = result.get(&(1970, 1)).unwrap();
        assert_eq!(snapshot.page_count, 1);
        assert_eq!(snapshot.merged_content, "Page content");
        assert_eq!(snapshot.merged_links.len(), 1);
    }

    #[test]
    fn test_merge_multi_page_same_date() {
        let pages = vec![
            make_page(
                "http://example.com/article?page=1",
                Some(1),
                100,
                "Page 1 content",
                vec!["http://link1.com".to_string()],
            ),
            make_page(
                "http://example.com/article?page=2",
                Some(2),
                100,
                "Page 2 content",
                vec!["http://link2.com".to_string()],
            ),
        ];

        let result = merge_pages_by_date(&pages);
        assert_eq!(result.len(), 1);

        let snapshot = result.get(&(1970, 1)).unwrap();
        assert_eq!(snapshot.page_count, 2);
        assert!(snapshot.merged_content.contains("Page 1 content"));
        assert!(snapshot.merged_content.contains("Page 2 content"));
        assert!(snapshot.merged_content.contains("---"));
        assert!(snapshot.merged_content.contains("## Page 2"));
        assert_eq!(snapshot.merged_links.len(), 2);
    }

    #[test]
    fn test_merge_deduplicates_links() {
        let shared_link = "http://shared.com".to_string();
        let pages = vec![
            make_page(
                "http://example.com/article?page=1",
                Some(1),
                100,
                "Page 1",
                vec![shared_link.clone(), "http://link1.com".to_string()],
            ),
            make_page(
                "http://example.com/article?page=2",
                Some(2),
                100,
                "Page 2",
                vec![shared_link, "http://link2.com".to_string()],
            ),
        ];

        let result = merge_pages_by_date(&pages);
        let snapshot = result.get(&(1970, 1)).unwrap();

        // Should have 3 unique links (shared appears once, link1 and link2 once each)
        assert_eq!(snapshot.merged_links.len(), 3);
        assert!(
            snapshot
                .merged_links
                .contains(&"http://shared.com".to_string())
        );
        assert!(
            snapshot
                .merged_links
                .contains(&"http://link1.com".to_string())
        );
        assert!(
            snapshot
                .merged_links
                .contains(&"http://link2.com".to_string())
        );
    }

    #[test]
    fn test_merge_multiple_dates() {
        // January 1, 1970 00:00:00 UTC = 0
        // February 1, 1970 00:00:00 UTC = 2678400 (31 days * 86400 seconds)
        let pages = vec![
            make_page(
                "http://example.com/article",
                None,
                0,
                "Content January",
                vec![],
            ),
            make_page(
                "http://example.com/article",
                None,
                2678400,
                "Content February",
                vec![],
            ),
        ];

        let result = merge_pages_by_date(&pages);
        assert_eq!(result.len(), 2);
        assert!(result.contains_key(&(1970, 1)));
        assert!(result.contains_key(&(1970, 2)));
    }

    #[test]
    fn test_merge_sorts_by_page_number() {
        // Intentionally add pages out of order
        let pages = vec![
            make_page(
                "http://example.com/article?page=3",
                Some(3),
                100,
                "Page 3",
                vec![],
            ),
            make_page(
                "http://example.com/article?page=1",
                Some(1),
                100,
                "Page 1",
                vec![],
            ),
            make_page(
                "http://example.com/article?page=2",
                Some(2),
                100,
                "Page 2",
                vec![],
            ),
        ];

        let result = merge_pages_by_date(&pages);
        let snapshot = result.get(&(1970, 1)).unwrap();

        // Content should be in order: Page 1, Page 2, Page 3
        let content_pos_1 = snapshot.merged_content.find("Page 1").unwrap();
        let content_pos_2 = snapshot.merged_content.find("Page 2").unwrap();
        let content_pos_3 = snapshot.merged_content.find("Page 3").unwrap();

        assert!(content_pos_1 < content_pos_2);
        assert!(content_pos_2 < content_pos_3);
    }

    #[test]
    fn test_merge_preserves_base_page_metadata() {
        let pages = vec![
            make_page(
                "http://example.com/article?page=1",
                Some(1),
                100,
                "Page 1",
                vec![],
            ),
            make_page(
                "http://example.com/article?page=2",
                Some(2),
                100,
                "Page 2",
                vec![],
            ),
        ];

        let result = merge_pages_by_date(&pages);
        let snapshot = result.get(&(1970, 1)).unwrap();

        // Base page should be the first one
        assert_eq!(snapshot.base_page.task.url_id, 1);
    }
}
