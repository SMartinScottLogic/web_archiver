use anyhow::Context as _;
use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Serialize, Serializer};
use simhash::simhash;
use std::collections::{HashSet, VecDeque};
use std::fs::{File, create_dir_all};
use std::path::PathBuf;

use crate::compressed_string;
use crate::types::{ExtractedPage, FetchTask, PageMetadata};

/// A snapshot of a page at a specific point in time.
/// Wraps all fields from ExtractedPage to preserve complete historical context.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoricalSnapshot {
    /// The fetch task metadata (url_id, url, depth, priority, discovered_from)
    pub task: FetchTask,
    /// Markdown-formatted content from the page
    pub content_markdown: HistoricalContentType,
    /// All links discovered on this snapshot
    /// Serialization is skipped as links are consolidated into HistoricalPage::all_links
    #[serde(skip_serializing)]
    pub links: Vec<String>,
    /// Metadata about the fetch (status code, content type, fetch time, title, etc.)
    pub metadata: Option<PageMetadata>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum HistoricalContentType {
    #[default]
    None,
    Literal(String),
    #[serde(with = "compressed_string")]
    Delta(String),
}

/// A page with its complete historical record.
/// Consolidates all snapshots for a given URL indexed by fetch time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoricalPage {
    /// The consolidated URL (normalized, without page params)
    pub url: String,
    /// The current (most up to date) snapshot for this URL
    pub current: Option<HistoricalSnapshot>,
    /// Historical snapshots for this URL, sorted by fetch_time (ascending)
    pub historical_snapshots: VecDeque<HistoricalSnapshot>,
    /// All unique links discovered across all snapshots (deduplicated)
    /// Serialized as a sorted JSON array for deterministic output
    #[serde(
        serialize_with = "serialize_sorted_links",
        deserialize_with = "deserialize_links"
    )]
    pub all_links: HashSet<String>,
}

impl HistoricalPage {
    /// Create a new HistoricalPage with default empty state
    pub fn new(url: String) -> Self {
        Self {
            url,
            current: None,
            historical_snapshots: VecDeque::new(),
            all_links: HashSet::new(),
        }
    }

    fn delta(
        current: &HistoricalSnapshot,
        snapshot: &HistoricalSnapshot,
    ) -> anyhow::Result<(u32, String)> {
        let last = match &current.content_markdown {
            HistoricalContentType::None => "",
            HistoricalContentType::Literal(t) => t.as_str(),
            HistoricalContentType::Delta(_) => panic!("Cannot delta based on a Delta"),
        };
        let new = match &snapshot.content_markdown {
            HistoricalContentType::None => "",
            HistoricalContentType::Literal(t) => t.as_str(),
            HistoricalContentType::Delta(_) => panic!("Cannot delta based on a Delta"),
        };
        let h1 = simhash(last);
        let h2 = simhash(new);
        let distance = simhash::hamming_distance(h1, h2);
        let mut delta = Vec::new();
        oxidelta::compress::encoder::encode_all(
            &mut delta,
            new.as_bytes(),
            last.as_bytes(),
            Default::default(),
        )?;
        let delta = general_purpose::STANDARD.encode(delta);
        Ok((distance, delta))
    }

    /// Add a snapshot to the historical record, maintaining sort order by fetch_time
    /// and automatically updating the deduplicated links set
    pub fn add_snapshot(&mut self, snapshot: HistoricalSnapshot) {
        // Verify that incoming snapshot is for later than current
        let snapshot_fetch_time = snapshot
            .metadata
            .as_ref()
            .map(|m| m.fetch_time)
            .unwrap_or_default();
        let current_fetch_time = match &self.current {
            None => 0,
            Some(current) => current
                .metadata
                .as_ref()
                .map(|m| m.fetch_time)
                .unwrap_or_default(),
        };
        assert!(snapshot_fetch_time > current_fetch_time);
        // Add snapshot's links to the set (deduplication is automatic), with paging stripped
        for link in &snapshot.links {
            let normalized_link = crate::url::remove_pagination_params(link);
            self.all_links.insert(normalized_link);
        }
        if let Some(current) = &self.current {
            let (distance, delta) = Self::delta(current, &snapshot).unwrap();
            if distance < 5 {
                // Replace 'current'
            } else {
                // Old current becomes a delta
                let mut current = current.to_owned();
                current.content_markdown = HistoricalContentType::Delta(delta);
                // Add snapshot, maintaining temporal order
                self.historical_snapshots.push_front(current);
            }
        }
        self.current = Some(snapshot);
    }

    /// Rebuild all_links from all snapshots by re-collecting and deduplicating.
    /// Useful if snapshots were modified externally or to ensure consistency.
    pub fn consolidate_links(&mut self) {
        self.all_links.clear();
        for snapshot in &self.historical_snapshots {
            for link in &snapshot.links {
                let normalized_link = crate::url::remove_pagination_params(link);
                self.all_links.insert(normalized_link);
            }
        }
    }

    /// Serialize this HistoricalPage to a JSON file with pretty formatting
    pub fn write_page(&self, path: &PathBuf) -> anyhow::Result<()> {
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

impl HistoricalSnapshot {
    /// Convert a single ExtractedPage into a HistoricalSnapshot
    pub fn from_extracted_page(page: ExtractedPage) -> Self {
        Self {
            task: page.task,
            content_markdown: match page.content_markdown {
                Some(t) => HistoricalContentType::Literal(t),
                None => HistoricalContentType::None,
            },
            links: page.links,
            metadata: page.metadata,
        }
    }
}

impl From<ExtractedPage> for HistoricalSnapshot {
    fn from(page: ExtractedPage) -> Self {
        HistoricalSnapshot::from_extracted_page(page)
    }
}

/// Custom serializer for HashSet<String> that outputs a sorted JSON array
fn serialize_sorted_links<S>(links: &HashSet<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut sorted: Vec<_> = links.iter().cloned().collect();
    sorted.sort();
    sorted.serialize(serializer)
}

/// Custom deserializer to convert JSON array into HashSet
fn deserialize_links<'de, D>(deserializer: D) -> Result<HashSet<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let links: Vec<String> = Vec::deserialize(deserializer)?;
    Ok(links.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_historical_page_creation() {
        let page = HistoricalPage::new("https://example.com".to_string());
        assert_eq!(page.url, "https://example.com");
        assert_eq!(page.historical_snapshots.len(), 0);
        assert_eq!(page.all_links.len(), 0);
    }

    #[test]
    fn test_add_snapshot() {
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
            links: vec!["https://link1.com".to_string()],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                title: Some("Page Title".to_string()),
                document_metadata: None,
            }),
        };

        page.add_snapshot(snapshot);
        assert!(page.current.is_some());
        assert_eq!(page.historical_snapshots.len(), 0);
    }

    #[test]
    fn test_snapshots_sorted_by_fetch_time() {
        let mut page = HistoricalPage::new("https://example.com".to_string());

        // Add snapshots in chronological order
        let snapshot_newest = HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal("Content v3".to_string()),
            links: vec![],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 3000,
                title: None,
                document_metadata: None,
            }),
        };

        let snapshot_newer = HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal("Content v2".to_string()),
            links: vec![],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 2000,
                title: None,
                document_metadata: None,
            }),
        };

        let snapshot_older = HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal("Content v1".to_string()),
            links: vec![],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 1000,
                title: None,
                document_metadata: None,
            }),
        };

        page.add_snapshot(snapshot_older);
        page.add_snapshot(snapshot_newer);
        page.add_snapshot(snapshot_newest);

        // Verify snapshots are sorted by fetch_time (descending)
        assert_eq!(
            3000,
            page.current.unwrap().metadata.as_ref().unwrap().fetch_time
        );
        assert_eq!(page.historical_snapshots.len(), 2);
        assert_eq!(
            page.historical_snapshots[0]
                .metadata
                .as_ref()
                .unwrap()
                .fetch_time,
            2000
        );
        assert_eq!(
            page.historical_snapshots[1]
                .metadata
                .as_ref()
                .unwrap()
                .fetch_time,
            1000
        );
    }

    #[test]
    #[should_panic]
    fn test_snapshots_out_of_order() {
        let mut page = HistoricalPage::new("https://example.com".to_string());

        // Add snapshots in reverse chronological order
        let snapshot_newer = HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal("Content v2".to_string()),
            links: vec![],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 2000,
                title: None,
                document_metadata: None,
            }),
        };

        let snapshot_older = HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal("Content v1".to_string()),
            links: vec![],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 1000,
                title: None,
                document_metadata: None,
            }),
        };

        page.add_snapshot(snapshot_newer);
        page.add_snapshot(snapshot_older);
        // Should PANIC
    }

    #[test]
    fn test_add_snapshot_strips_pagination_from_links() {
        let mut page = HistoricalPage::new("https://example.com".to_string());

        let snapshot = HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::None,
            links: vec![
                "https://example.com/article?page=2".to_string(),
                "https://example.com/article?page=3".to_string(),
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
        assert_eq!(page.all_links.len(), 1);
        assert!(page.all_links.contains("https://example.com/article"));
    }

    #[test]
    fn test_consolidate_links_deduplicates() {
        let mut page = HistoricalPage::new("https://example.com".to_string());

        let snapshot1 = HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::None,
            links: vec![
                "https://link1.com".to_string(),
                "https://link2.com".to_string(),
                "https://link1.com".to_string(), // duplicate
            ],
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
            content_markdown: HistoricalContentType::None,
            links: vec![
                "https://link2.com".to_string(), // duplicate from snapshot1
                "https://link3.com".to_string(),
            ],
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

        // all_links should be automatically deduplicated when snapshots are added
        assert_eq!(page.all_links.len(), 3);
        assert!(page.all_links.contains("https://link1.com"));
        assert!(page.all_links.contains("https://link2.com"));
        assert!(page.all_links.contains("https://link3.com"));
    }

    #[test]
    fn test_snapshot_from_extracted_page() {
        let extracted = ExtractedPage {
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
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                title: Some("Title".to_string()),
                document_metadata: None,
            }),
        };

        let snapshot = HistoricalSnapshot::from_extracted_page(extracted.clone());

        assert_eq!(snapshot.task.url_id, 1);
        assert_eq!(snapshot.task.url, "https://example.com");
        assert_eq!(
            snapshot.content_markdown,
            HistoricalContentType::Literal("Content".to_string())
        );
        assert_eq!(snapshot.links.len(), 1);
        assert_eq!(snapshot.metadata.as_ref().unwrap().status_code, 200);
    }

    #[test]
    fn test_from_trait_for_snapshot() {
        let extracted = ExtractedPage {
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

        let snapshot: HistoricalSnapshot = extracted.into();
        assert_eq!(snapshot.task.url_id, 1);
    }

    #[test]
    fn test_historical_page_serialization_skips_snapshot_links() {
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
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                title: Some("Title".to_string()),
                document_metadata: None,
            }),
        };

        page.add_snapshot(snapshot);

        // Serialize to JSON
        let json = serde_json::to_string(&page).expect("Failed to serialize");

        // Verify JSON doesn't contain "links" field (which would be in snapshots)
        // It should only have the consolidated all_links
        assert!(
            json.contains("\"all_links\""),
            "JSON should contain all_links"
        );

        // Parse JSON to verify structure
        let json_value: serde_json::Value =
            serde_json::from_str(&json).expect("Failed to parse JSON");

        // Snapshots should not have a links field
        let snapshots = json_value["historical_snapshots"]
            .as_array()
            .expect("Should have snapshots");
        assert_eq!(snapshots.len(), 0, "Should have no historical snapshots");
        let current = json_value["current"]
            .as_object()
            .expect("should have current");

        assert!(
            current.get("links").is_none(),
            "Snapshot should not serialize links field"
        );
        assert!(
            current.get("content_markdown").is_some(),
            "Snapshot should have content_markdown"
        );
        assert!(current.get("task").is_some(), "Snapshot should have task");
        assert!(
            current.get("metadata").is_some(),
            "Snapshot should have metadata"
        );
    }

    #[test]
    fn test_delta_identical() {
        let mut page = HistoricalPage::new("https://example.com".to_string());

        page.add_snapshot(HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal(
                "The cat sat on the mat and purred.".to_string(),
            ),
            links: vec![],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                title: Some("Title".to_string()),
                document_metadata: None,
            }),
        });

        let snapshot = HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal(
                "The cat sat on the mat and purred.".to_string(),
            ),
            links: vec![],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 2000,
                title: Some("Title".to_string()),
                document_metadata: None,
            }),
        };

        let (distance, delta) = HistoricalPage::delta(&page.current.unwrap(), &snapshot).unwrap();

        assert_eq!(0, distance);
        assert_eq!(28, delta.len());
    }

    #[test]
    fn test_delta_similar() {
        let mut page = HistoricalPage::new("https://example.com".to_string());

        page.add_snapshot(HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal(
                "The cat sat on the mat and purred.".to_string(),
            ),
            links: vec![],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                title: Some("Title".to_string()),
                document_metadata: None,
            }),
        });

        let snapshot = HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal(
                "The cat slept on the mat and purred.".to_string(),
            ),
            links: vec![],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 2000,
                title: Some("Title".to_string()),
                document_metadata: None,
            }),
        };

        let (distance, delta) = HistoricalPage::delta(&page.current.unwrap(), &snapshot).unwrap();

        assert_eq!(6, distance);
        assert_eq!(36, delta.len());
    }

    #[test]
    fn test_delta_far() {
        let mut page = HistoricalPage::new("https://example.com".to_string());

        page.add_snapshot(HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal(
                "The cat sat on the mat and purred.".to_string(),
            ),
            links: vec![],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                title: Some("Title".to_string()),
                document_metadata: None,
            }),
        });

        let snapshot = HistoricalSnapshot {
            task: FetchTask {
                url_id: 1,
                url: "https://example.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: HistoricalContentType::Literal(
                "Once upon a time, there was a cat on a mat.".to_string(),
            ),
            links: vec![],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 2000,
                title: Some("Title".to_string()),
                document_metadata: None,
            }),
        };

        let (distance, delta) = HistoricalPage::delta(&page.current.unwrap(), &snapshot).unwrap();

        assert_eq!(33, distance);
        assert_eq!(72, delta.len());
    }
}
