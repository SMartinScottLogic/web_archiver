use std::collections::HashMap;
use std::ops::Add;
use std::time::{Duration, SystemTime};

use common::Archiver;
use common::historical::{HistoricalContent, HistoricalPage, HistoricalSnapshot};
use common::types::{FetchTask, Priority};

use chrono::offset::Utc;
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use itertools::Itertools;

use crate::aggregator::AggregateKey;
use crate::multi_page_merger::MergedSnapshot;

/// Convert Unix timestamp to (year, month) tuple
fn timestamp_to_year_month(fetch_time: u64) -> (u32, u32) {
    let fetch_time = SystemTime::UNIX_EPOCH.add(Duration::from_secs(fetch_time));
    let fetch_time: DateTime<Utc> = fetch_time.into();
    (fetch_time.year() as u32, fetch_time.month())
}

/// Convert (year, month) tuple to epoch seconds for the first day of the month
fn timestamp_to_year_month_inverse((year, month): (u32, u32)) -> u64 {
    let nd = NaiveDate::from_ymd_opt(year as i32, month, 1);
    let nd = nd.unwrap();
    let nt = NaiveTime::from_hms_opt(0, 0, 0);
    let nt = nt.unwrap();
    NaiveDateTime::new(nd, nt).and_utc().timestamp() as u64
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
pub struct HistoricalSerializer<T: Archiver> {
    /// Archiver to use to add articles to the document store
    archiver: T,
}

impl<T: Archiver> HistoricalSerializer<T> {
    pub fn new(archiver: T) -> Self {
        Self { archiver }
    }

    // /// Converts a MergedSnapshot back to an ExtractedPage-like structure suitable for HistoricalSnapshot.
    // /// The merged_content replaces the original content_markdown.
    // fn merged_snapshot_to_historical_snapshot_old(
    //     merged_snapshot: &MergedSnapshot,
    //     year_month: (u32, u32),
    // ) -> HistoricalSnapshot {
    //     let mut base_page = merged_snapshot.base_page.clone();
    //     // Replace content with merged content
    //     base_page.content_markdown = Some(merged_snapshot.content.clone());
    //     // Replace links with merged links
    //     base_page.links = merged_snapshot.all_links.iter().cloned().collect();
    //     // Update fetch_time metadata to reflect the month
    //     if let Some(ref mut metadata) = base_page.metadata {
    //         // Keep the original fetch_time but mark it as a merged snapshot
    //         // The year_month tuple is implicit in the grouping
    //         metadata.fetch_time = timestamp_to_year_month_inverse(year_month);
    //     }

    //     HistoricalSnapshot::from_extracted_page(base_page)
    // }

    fn merged_snapshot_to_historical_snapshot(
        merged_snapshot: &MergedSnapshot,
        year_month: (u32, u32),
    ) -> HistoricalSnapshot {
        let content_markdown = merged_snapshot
            .content
            .iter()
            .map(|snapshot_page| HistoricalContent {
                page: snapshot_page.page,
                content: common::historical::HistoricalContentType::Literal(
                    snapshot_page.content.clone(),
                ),
            })
            .collect();
        let links = merged_snapshot.all_links.iter().cloned().collect();
        let metadata = merged_snapshot
            .base_page
            .metadata
            .clone()
            .map(|mut metadata| {
                metadata.fetch_time = timestamp_to_year_month_inverse(year_month);
                metadata
            });
        HistoricalSnapshot {
            content_markdown,
            links,
            metadata,
        }
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
            let fetch_task = FetchTask {
                url: key.normalized_url.clone(),
                article_id: 0,
                url_id: 0,
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
            };
            let mut page = HistoricalPage::new(fetch_task);

            // Add each merged snapshot to the historical page
            for merged_snapshot in merged_snapshots.iter().sorted_by_cached_key(|h| {
                h.base_page
                    .metadata
                    .as_ref()
                    .map(|m| m.fetch_time)
                    .unwrap_or_default()
            }) {
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

            // Serialize to disk
            self.archiver.store_page(&page)?;

            files_written += 1;
        }

        Ok(files_written)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::multi_page_merger::SnapshotPage;

    use super::*;
    use common::{
        DefaultArchiver, historical::HistoricalContentType, types::FetchTask, url::hash_url,
    };
    use rebuild_archive::extracted_page::ExtractedPage;

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
    fn test_url_hash_consistency() {
        // Same URL should always produce same hash
        let url = "https://example.com/test/page";
        let hash1 = hash_url(url);
        let hash2 = hash_url(url);
        assert_eq!(hash1, hash2);

        // Different URLs should produce different hashes
        let hash3 = hash_url("https://example.com/test/page2");
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_merged_snapshot_to_historical_snapshot() {
        use crate::multi_page_merger::MergedSnapshot;
        use common::types::PageMetadata;

        let base_page = ExtractedPage {
            task: FetchTask {
                article_id: 0,
                url_id: 1,
                url: "http://example.com/page".to_string(),
                depth: 1,
                priority: Priority::default(),
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
            content: vec![SnapshotPage {
                page: 1,
                content: "Merged content from pages 1 and 2".to_string(),
            }],
            all_links: HashSet::from([
                "http://link1.com".to_string(),
                "http://link2.com".to_string(),
            ]),
            page_count: 2,
        };

        let snapshot =
            HistoricalSerializer::<DefaultArchiver>::merged_snapshot_to_historical_snapshot(
                &merged_snapshot,
                (1970, 2),
            );

        assert_eq!(
            snapshot.content_markdown,
            vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal(
                    "Merged content from pages 1 and 2".to_string()
                )
            }]
        );
        assert_eq!(snapshot.links.len(), 2);
        assert!(snapshot.links.contains("http://link1.com"));
        assert!(snapshot.links.contains("http://link2.com"));
    }
}
