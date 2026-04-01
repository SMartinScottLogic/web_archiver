use std::collections::HashMap;
use std::path::PathBuf;

use common::historical::{HistoricalPage, HistoricalSnapshot};

use crate::aggregator::AggregateKey;
use crate::multi_page_merger::MergedSnapshot;

/// Convert Unix timestamp to (year, month) tuple
fn timestamp_to_year_month(fetch_time: u64) -> (u32, u32) {
    let days_since_epoch = fetch_time / 86400;
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

/// Convert (year, month) tuple to epoch seconds for the first day of the month
fn timestamp_to_year_month_inverse(year_month: (u32, u32)) -> u64 {
    let mut days = 0u64;

    // Count days for all years before year_month.0
    for y in 1970..year_month.0 {
        days += if is_leap_year(y) { 366 } else { 365 };
    }

    // Count days for all months before year_month.1
    let is_leap = is_leap_year(year_month.0);
    let days_in_months = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    for m in 1..year_month.1 {
        days += days_in_months[(m - 1) as usize] as u64;
    }

    days * 86400 // Convert to seconds
}

pub fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Phase 2e: Convert merged snapshots to HistoricalPage format and serialize to disk
///
/// This module handles:
/// - Converting MergedSnapshot objects to HistoricalSnapshot format
/// - Creating HistoricalPage objects for each URL/domain combination
/// - Consolidating links across all snapshots
/// - Serializing to JSON files in the target directory structure
pub struct HistoricalSerializer {
    /// Target directory where HistoricalPage JSON files will be written
    pub target_dir: PathBuf,
}

impl HistoricalSerializer {
    pub fn new(target_dir: impl Into<PathBuf>) -> Self {
        Self {
            target_dir: target_dir.into(),
        }
    }

    /// Converts a MergedSnapshot back to an ExtractedPage-like structure suitable for HistoricalSnapshot.
    /// The merged_content replaces the original content_markdown.
    fn merged_snapshot_to_historical_snapshot(
        merged_snapshot: &MergedSnapshot,
        year_month: (u32, u32),
    ) -> HistoricalSnapshot {
        let mut base_page = merged_snapshot.base_page.clone();
        // Replace content with merged content
        base_page.content_markdown = Some(merged_snapshot.merged_content.clone());
        // Replace links with merged links
        base_page.links = merged_snapshot.merged_links.clone();
        // Update fetch_time metadata to reflect the month
        if let Some(ref mut metadata) = base_page.metadata {
            // Keep the original fetch_time but mark it as a merged snapshot
            // The year_month tuple is implicit in the grouping
            metadata.fetch_time = timestamp_to_year_month_inverse(year_month);
        }

        HistoricalSnapshot::from_extracted_page(base_page)
    }

    /// Serialize all historical pages to the target directory.
    ///
    /// Returns the number of files written.
    pub fn serialize_all(
        &self,
        aggregates: &HashMap<AggregateKey, Vec<MergedSnapshot>>,
    ) -> anyhow::Result<usize> {
        let mut files_written = 0;

        for (key, merged_snapshots) in aggregates {
            // Create a HistoricalPage for this domain+URL combination
            let mut page = HistoricalPage::new(key.normalized_url.clone());

            // Add each merged snapshot to the historical page
            for merged_snapshot in merged_snapshots {
                // Extract year_month from base_metadata
                let year_month = if let Some(metadata) = &merged_snapshot.base_page.metadata {
                    timestamp_to_year_month(metadata.fetch_time)
                } else {
                    (1970, 1) // fallback
                };

                let snapshot =
                    Self::merged_snapshot_to_historical_snapshot(merged_snapshot, year_month);
                page.add_snapshot(snapshot);
            }

            // Consolidate all links
            page.consolidate_links();

            // Generate output path: target_dir/{domain}/historical.json
            let output_path = self.generate_output_path(&key.domain);

            // Serialize to disk
            page.write_page(&output_path)?;
            files_written += 1;
        }

        Ok(files_written)
    }

    /// Generate output path for a historical page based on domain.
    /// Pattern: {target_dir}/{domain}/historical.json
    fn generate_output_path(&self, domain: &str) -> PathBuf {
        self.target_dir.join(domain).join("historical.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::types::FetchTask;

    #[test]
    fn test_year_month_to_timestamp_epoch() {
        // (1970, 1) should map to 0 (epoch)
        assert_eq!(timestamp_to_year_month_inverse((1970, 1)), 0);
    }

    #[test]
    fn test_year_month_to_timestamp_february() {
        // (1970, 2) should be 31 days * 86400 seconds
        assert_eq!(timestamp_to_year_month_inverse((1970, 2)), 2678400);
    }

    #[test]
    fn test_timestamp_to_year_month_roundtrip() {
        // Test roundtrip conversion for January
        let (year, month) = timestamp_to_year_month(0);
        assert_eq!((year, month), (1970, 1));

        // Test roundtrip for February
        let (year, month) = timestamp_to_year_month(2678400);
        assert_eq!((year, month), (1970, 2));
    }

    #[test]
    fn test_historical_serializer_creation() {
        let serializer = HistoricalSerializer::new("/tmp/test");
        assert_eq!(serializer.target_dir, PathBuf::from("/tmp/test"));
    }

    #[test]
    fn test_output_path_generation() {
        let serializer = HistoricalSerializer::new("/tmp/test");
        let path = serializer.generate_output_path("example.com");
        assert_eq!(path, PathBuf::from("/tmp/test/example.com/historical.json"));
    }

    #[test]
    fn test_merged_snapshot_to_historical_snapshot() {
        use crate::multi_page_merger::MergedSnapshot;
        use common::types::PageMetadata;

        let base_page = common::types::ExtractedPage {
            task: FetchTask {
                url_id: 1,
                url: "http://example.com/page".to_string(),
                depth: 1,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: Some("Original content".to_string()),
            links: vec!["http://link1.com".to_string()],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 2678400, // Feb 1, 1970
                title: Some("Test Page".to_string()),
                document_metadata: None,
            }),
        };

        let merged_snapshot = MergedSnapshot {
            base_page,
            merged_content: "Merged content from pages 1 and 2".to_string(),
            merged_links: vec![
                "http://link1.com".to_string(),
                "http://link2.com".to_string(),
            ],
            page_count: 2,
        };

        let snapshot = HistoricalSerializer::merged_snapshot_to_historical_snapshot(
            &merged_snapshot,
            (1970, 2),
        );

        assert_eq!(
            snapshot.content_markdown,
            Some("Merged content from pages 1 and 2".to_string())
        );
        assert_eq!(snapshot.links.len(), 2);
        assert!(snapshot.links.contains(&"http://link1.com".to_string()));
        assert!(snapshot.links.contains(&"http://link2.com".to_string()));
    }
}
