use anyhow::Context as _;
use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{File, create_dir_all};
use std::path::Path;
use tracing::{debug, error};

use crate::compressed_string;
use crate::types::{FetchTask, PageMetadata};

/// A snapshot of a page at a specific point in time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoricalSnapshot {
    /// Markdown-formatted content from the page
    pub content_markdown: Vec<HistoricalContent>,
    /// All links discovered on this snapshot
    /// Serialization is skipped as links are consolidated into HistoricalPage::all_links
    #[serde(skip)]
    pub links: HashSet<String>,
    /// Metadata about the fetch (status code, content type, fetch time, title, etc.)
    pub metadata: Option<PageMetadata>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoricalContent {
    pub content: HistoricalContentType,
    pub page: u32,
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
    /// The fetch task metadata (url_id, url, depth, priority, discovered_from)
    pub task: FetchTask,
    /// The current (most up to date) snapshot for this URL
    pub current: Option<HistoricalSnapshot>,
    /// Historical snapshots for this URL, sorted by fetch_time (ascending)
    #[serde(default)]
    pub historical_snapshots: VecDeque<HistoricalSnapshot>,
    /// All unique links discovered across all snapshots (deduplicated)
    /// Serialized as a sorted JSON array for deterministic output
    #[serde(
        serialize_with = "serialize_sorted_links",
        deserialize_with = "deserialize_links"
    )]
    pub all_links: HashSet<String>,
    /// Historical fetch times (all, including duplicates)
    #[serde(default)]
    pub history: VecDeque<u64>,
}
impl HistoricalPage {
    /// Create a new HistoricalPage with default empty state
    pub fn new(task: FetchTask) -> Self {
        Self {
            task,
            current: None,
            historical_snapshots: Default::default(),
            all_links: Default::default(),
            history: Default::default(),
        }
    }

    fn page_map(snapshot: &HistoricalSnapshot) -> anyhow::Result<HashMap<u32, &str>> {
        let mut result = HashMap::new();
        for page in &snapshot.content_markdown {
            let text = match &page.content {
                HistoricalContentType::None => "",
                HistoricalContentType::Literal(t) => t.as_str(),
                HistoricalContentType::Delta(_) => {
                    return Err(anyhow::Error::msg("cannot delta based on a Delta"));
                }
            };
            if let Some(other) = result.insert(page.page, text) {
                error!("snapshot: {:?}", snapshot);
                error!(
                    "Expected {} to be a new page, but already existed with content: {}, wanted to insert {}",
                    page.page, other, text
                );
                return Err(anyhow::Error::msg("cannot add a duplicate page"));
            };
        }
        Ok(result)
    }

    fn delta(
        current: &HistoricalSnapshot,
        snapshot: &HistoricalSnapshot,
    ) -> anyhow::Result<Vec<(u32, (u32, String))>> {
        let current = Self::page_map(current)?;
        let snapshot = Self::page_map(snapshot)?;

        let max_page = std::cmp::max(
            current.keys().cloned().max().unwrap_or_default(),
            snapshot.keys().cloned().max().unwrap_or_default(),
        );
        let min_page = std::cmp::min(
            current.keys().cloned().min().unwrap_or_default(),
            snapshot.keys().cloned().min().unwrap_or_default(),
        );
        let mut deltas = Vec::new();
        for page in min_page..=max_page {
            let c = current.get(&page).copied().unwrap_or_default();
            let s = snapshot.get(&page).copied().unwrap_or_default();
            let delta = Self::delta_page(c, s)?;
            debug!(c, s, page, ?delta, "page delta");
            deltas.push((page, delta));
        }
        Ok(deltas)
    }

    fn delta_page(last: &str, new: &str) -> anyhow::Result<(u32, String)> {
        let distance = if last.cmp(new).is_eq() { 0 } else { 100 };
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
    pub fn add_snapshot(&mut self, snapshot: HistoricalSnapshot) -> anyhow::Result<()> {
        // Verify that incoming snapshot is for later than current
        let snapshot_fetch_time = snapshot.metadata.as_ref().map(|m| m.fetch_time);
        let current_fetch_time = self
            .current
            .as_ref()
            .and_then(|current| current.metadata.as_ref().map(|m| m.fetch_time));
        if let (Some(current_time), Some(snapshot_time)) = (current_fetch_time, snapshot_fetch_time)
        {
            assert!(
                snapshot_time > current_time,
                "snapshot_time: {}, current_time: {}",
                snapshot_time,
                current_time
            )
        };
        // Add snapshot's links to the set (deduplication is automatic), with paging stripped
        for link in &snapshot.links {
            let normalized_link = crate::url::remove_pagination_params(link);
            self.all_links.insert(normalized_link);
        }

        debug!(?self.all_links, self.task.url, "links");
        if let Some(current) = &self.current {
            let deltas = Self::delta(current, &snapshot)?;
            let max_distance = deltas
                .iter()
                .map(|(_page, (distance, _delta))| distance)
                .cloned()
                .max()
                .unwrap_or_default();
            if max_distance < 5 {
                // Replace 'current'
                debug!(
                    "snapshots very similar, replacing current: {}; {:?} | {:?} | {:?}",
                    max_distance, deltas, current, snapshot
                );
            } else {
                // Old current becomes a delta
                let mut current = current.to_owned();
                let mut delta = deltas
                    .into_iter()
                    .map(|(page, (_distance, delta))| HistoricalContent {
                        page,
                        content: HistoricalContentType::Delta(delta),
                    })
                    .collect::<Vec<_>>();
                delta.sort_by_cached_key(|c| c.page);
                current.content_markdown = delta;
                // Add snapshot, maintaining temporal order
                self.historical_snapshots.push_front(current);
            }
        }
        self.current = Some(snapshot);
        if let Some(incoming_fetch_time) = snapshot_fetch_time {
            self.history.push_front(incoming_fetch_time);
        }
        Ok(())
    }

    /// Rebuild all_links from all snapshots by re-collecting and deduplicating.
    /// Useful if snapshots were modified externally or to ensure consistency.
    pub fn consolidate_links(&mut self) {
        self.all_links.clear();
        for current in self.current.iter() {
            for link in &current.links {
                let normalized_link = crate::url::remove_pagination_params(link);
                self.all_links.insert(normalized_link);
            }
        }
        for snapshot in &self.historical_snapshots {
            for link in &snapshot.links {
                let normalized_link = crate::url::remove_pagination_params(link);
                self.all_links.insert(normalized_link);
            }
        }
    }

    /// Serialize this HistoricalPage to a JSON file with pretty formatting
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
    use crate::types::Priority;

    use super::*;

    fn default_task(url: &str) -> FetchTask {
        FetchTask {
            article_id: 0,
            url_id: 0,
            url: url.to_string(),
            depth: 0,
            priority: Priority::default(),
            discovered_from: None,
        }
    }
    #[test]
    fn test_historical_page_creation() {
        let page = HistoricalPage::new(default_task("https://example.com"));
        assert_eq!(page.task.url, "https://example.com");
        assert_eq!(page.historical_snapshots.len(), 0);
        assert_eq!(page.all_links.len(), 0);
    }

    #[test]
    fn test_add_snapshot() {
        let mut page = HistoricalPage::new(default_task("https://example.com"));

        let snapshot = HistoricalSnapshot {
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal("Content".to_string()),
            }],
            links: HashSet::from(["https://link1.com".to_string()]),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                authors: Vec::new(),
                title: Some("Page Title".to_string()),
                document_metadata: None,
                json_ld: None,
            }),
        };

        page.add_snapshot(snapshot).unwrap();
        assert!(page.current.is_some());
        assert_eq!(page.historical_snapshots.len(), 0);
    }

    #[test]
    fn test_snapshots_sorted_by_fetch_time() {
        let mut page = HistoricalPage::new(default_task("https://example.com"));

        // Add snapshots in chronological order
        let snapshot_newest = HistoricalSnapshot {
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal("Content v3".to_string()),
            }],
            links: HashSet::new(),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 3000,
                authors: Vec::new(),
                title: None,
                document_metadata: None,
                json_ld: None,
            }),
        };

        let snapshot_newer = HistoricalSnapshot {
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal("Content v2".to_string()),
            }],
            links: HashSet::new(),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 2000,
                authors: Vec::new(),
                title: None,
                document_metadata: None,
                json_ld: None,
            }),
        };

        let snapshot_older = HistoricalSnapshot {
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal("Content v1".to_string()),
            }],
            links: HashSet::new(),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 1000,
                authors: Vec::new(),
                title: None,
                document_metadata: None,
                json_ld: None,
            }),
        };

        page.add_snapshot(snapshot_older).unwrap();
        page.add_snapshot(snapshot_newer).unwrap();
        page.add_snapshot(snapshot_newest).unwrap();

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
        let mut page = HistoricalPage::new(default_task("https://example.com"));

        // Add snapshots in reverse chronological order
        let snapshot_newer = HistoricalSnapshot {
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal("Content v2".to_string()),
            }],
            links: HashSet::new(),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 2000,
                authors: Vec::new(),
                title: None,
                document_metadata: None,
                json_ld: None,
            }),
        };

        let snapshot_older = HistoricalSnapshot {
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal("Content v1".to_string()),
            }],
            links: HashSet::new(),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 1000,
                authors: Vec::new(),
                title: None,
                document_metadata: None,
                json_ld: None,
            }),
        };

        page.add_snapshot(snapshot_newer).unwrap();
        page.add_snapshot(snapshot_older).unwrap();
        // Should PANIC
    }

    #[test]
    fn test_add_snapshot_strips_pagination_from_links() {
        let mut page = HistoricalPage::new(default_task("https://example.com"));

        let snapshot = HistoricalSnapshot {
            content_markdown: Vec::new(),
            links: HashSet::from([
                "https://example.com/article?page=2".to_string(),
                "https://example.com/article?page=3".to_string(),
            ]),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 1000,
                authors: Vec::new(),
                title: None,
                document_metadata: None,
                json_ld: None,
            }),
        };

        page.add_snapshot(snapshot).unwrap();
        assert_eq!(page.all_links.len(), 1);
        assert!(page.all_links.contains("https://example.com/article"));
    }

    #[test]
    fn test_consolidate_links_deduplicates() {
        let mut page = HistoricalPage::new(default_task("https://example.com"));

        let snapshot1 = HistoricalSnapshot {
            content_markdown: Vec::new(),
            links: HashSet::from([
                "https://link1.com".to_string(),
                "https://link2.com".to_string(),
                "https://link1.com".to_string(), // duplicate
            ]),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 1000,
                authors: Vec::new(),
                title: None,
                document_metadata: None,
                json_ld: None,
            }),
        };

        let snapshot2 = HistoricalSnapshot {
            content_markdown: Vec::new(),
            links: HashSet::from([
                "https://link2.com".to_string(), // duplicate from snapshot1
                "https://link3.com".to_string(),
            ]),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: None,
                fetch_time: 2000,
                authors: Vec::new(),
                title: None,
                document_metadata: None,
                json_ld: None,
            }),
        };

        page.add_snapshot(snapshot1).unwrap();
        page.add_snapshot(snapshot2).unwrap();

        // all_links should be automatically deduplicated when snapshots are added
        assert_eq!(page.all_links.len(), 3);
        assert!(page.all_links.contains("https://link1.com"));
        assert!(page.all_links.contains("https://link2.com"));
        assert!(page.all_links.contains("https://link3.com"));
    }

    #[test]
    fn test_historical_page_serialization_skips_snapshot_links() {
        let mut page = HistoricalPage::new(default_task("https://example.com"));

        let snapshot = HistoricalSnapshot {
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
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                authors: Vec::new(),
                title: Some("Title".to_string()),
                document_metadata: None,
                json_ld: None,
            }),
        };

        page.add_snapshot(snapshot).unwrap();

        // Serialize to JSON
        let json = serde_json::to_string(&page).expect("Failed to serialize");
        println!("json: {json}");
        // Verify JSON doesn't contain "links" field (which would be in snapshots)
        // It should only have the consolidated all_links
        assert!(
            json.contains("\"all_links\""),
            "JSON should contain all_links"
        );

        // Parse JSON to verify structure
        let json_value: serde_json::Value =
            serde_json::from_str(&json).expect("Failed to parse JSON");

        // Task should be top-level
        assert!(json_value.get("task").is_some(), "Should have task");
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
        assert!(
            current.get("metadata").is_some(),
            "Snapshot should have metadata"
        );
    }

    #[test]
    fn test_delta_identical() {
        let mut page = HistoricalPage::new(default_task("https://example.com"));

        page.add_snapshot(HistoricalSnapshot {
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal(
                    "The cat sat on the mat and purred.".to_string(),
                ),
            }],
            links: HashSet::new(),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                authors: Vec::new(),
                title: Some("Title".to_string()),
                document_metadata: None,
                json_ld: None,
            }),
        })
        .unwrap();

        let snapshot = HistoricalSnapshot {
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal(
                    "The cat sat on the mat and purred.".to_string(),
                ),
            }],
            links: HashSet::new(),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 2000,
                authors: Vec::new(),
                title: Some("Title".to_string()),
                document_metadata: None,
                json_ld: None,
            }),
        };

        let deltas = HistoricalPage::delta(&page.current.unwrap(), &snapshot).unwrap();

        assert_eq!(1, deltas.len());
        let (_page, (distance, delta)) = deltas.first().unwrap().to_owned();
        assert_eq!(0, distance);
        assert_eq!(28, delta.len());
    }

    #[test]
    fn test_delta_similar() {
        let mut page = HistoricalPage::new(default_task("https://example.com"));

        page.add_snapshot(HistoricalSnapshot {
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal(
                    "The cat sat on the mat and purred.".to_string(),
                ),
            }],
            links: HashSet::new(),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                authors: Vec::new(),
                title: Some("Title".to_string()),
                document_metadata: None,
                json_ld: None,
            }),
        })
        .unwrap();

        let snapshot = HistoricalSnapshot {
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal(
                    "The cat slept on the mat and purred.".to_string(),
                ),
            }],
            links: HashSet::new(),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 2000,
                authors: Vec::new(),
                title: Some("Title".to_string()),
                document_metadata: None,
                json_ld: None,
            }),
        };

        let deltas = HistoricalPage::delta(&page.current.unwrap(), &snapshot).unwrap();

        assert_eq!(1, deltas.len());
        let (_page, (distance, delta)) = deltas.first().unwrap().to_owned();

        assert_eq!(100, distance);
        assert_eq!(36, delta.len());
    }

    #[test]
    fn test_delta_far() {
        let mut page = HistoricalPage::new(default_task("https://example.com"));

        page.add_snapshot(HistoricalSnapshot {
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal(
                    "The cat sat on the mat and purred.".to_string(),
                ),
            }],
            links: HashSet::new(),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                authors: Vec::new(),
                title: Some("Title".to_string()),
                document_metadata: None,
                json_ld: None,
            }),
        })
        .unwrap();

        let snapshot = HistoricalSnapshot {
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: HistoricalContentType::Literal(
                    "Once upon a time, there was a cat on a mat.".to_string(),
                ),
            }],
            links: HashSet::new(),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 2000,
                authors: Vec::new(),
                title: Some("Title".to_string()),
                document_metadata: None,
                json_ld: None,
            }),
        };

        let deltas = HistoricalPage::delta(&page.current.unwrap(), &snapshot).unwrap();

        assert_eq!(1, deltas.len());
        let (_page, (distance, delta)) = deltas.first().unwrap().to_owned();

        assert_eq!(100, distance);
        assert_eq!(72, delta.len());
    }
}
