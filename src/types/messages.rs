#![allow(dead_code)]

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
    pub metadata: PageMetadata,
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
}
