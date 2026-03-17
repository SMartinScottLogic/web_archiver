#![allow(dead_code)]

use std::collections::HashMap;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FetchTask {
    pub url_id: i64,
    pub url: String,

    pub depth: u32,
    pub priority: i32,

    pub discovered_from: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct FetchedPage {
    pub task: FetchTask,
    pub status_code: u16,
    pub content_type: Option<String>,
    pub fetch_time: u64,
    pub body: std::sync::Arc<Vec<u8>>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ExtractedPage {
    pub task: FetchTask,
    pub content_markdown: Option<String>,
    pub links: Vec<String>,
    pub metadata: Option<PageMetadata>,
}

#[derive(Clone, Debug)]
pub struct DiscoveredLinks {
    pub parent_url_id: i64,
    pub links: Vec<String>,
    pub depth: u32,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PageMetadata {
    pub status_code: u16,
    pub content_type: Option<String>,
    pub fetch_time: u64,
    pub title: Option<String>,
    pub document_metadata: Option<Vec<HashMap<String, String>>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetch_task_clone_eq() {
        let t1 = FetchTask {
            url_id: 1,
            url: "http://foo.com".to_string(),
            depth: 0,
            priority: 1,
            discovered_from: None,
        };
        let t2 = t1.clone();
        assert_eq!(t1.url, t2.url);
        assert_eq!(t1.url_id, t2.url_id);
    }

    #[test]
    fn test_page_metadata_fields() {
        let meta = PageMetadata {
            status_code: 200,
            content_type: Some("text/html".to_string()),
            fetch_time: 123,
            title: Some("Title".to_string()),
            document_metadata: Some(vec![]),
        };
        assert_eq!(meta.status_code, 200);
        assert_eq!(meta.content_type.as_deref(), Some("text/html"));
        assert_eq!(meta.title.as_deref(), Some("Title"));
    }

    #[test]
    fn test_discovered_links() {
        let links = DiscoveredLinks {
            parent_url_id: 1,
            links: vec!["a".to_string(), "b".to_string()],
            depth: 2,
        };
        assert_eq!(links.links.len(), 2);
        assert_eq!(links.depth, 2);
    }
}
