use common::types::{FetchTask, Priority};

pub mod parser;
pub mod router;

#[derive(Clone, Debug, PartialEq)]
pub struct FetchedPage {
    pub task: FetchTask,
    pub status_code: u16,
    pub content_type: Option<String>,
    pub fetch_time: u64,
    pub body: std::sync::Arc<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct DiscoveredLinks {
    pub parent_url_id: i64,
    pub links: Vec<DiscoveredLink>,
    pub depth: u32,
}

#[derive(Clone, Debug)]
pub struct DiscoveredLink {
    pub priority: Priority,
    pub url: String,
}
impl From<&str> for DiscoveredLink {
    fn from(value: &str) -> Self {
        Self {
            url: value.to_string(),
            priority: Priority::Normal,
        }
    }
}

#[cfg(test)]
mod tests {
    use common::types::Priority;

    use super::*;

    fn sample_task() -> FetchTask {
        FetchTask {
            article_id: 0,
            url_id: 42,
            url: "http://example.com".to_string(),
            depth: 1,
            priority: Priority::default(),
            discovered_from: Some(1),
        }
    }

    #[test]
    fn test_discovered_links() {
        let links = DiscoveredLinks {
            parent_url_id: 1,
            links: vec!["a".into(), "b".into()],
            depth: 2,
        };
        assert_eq!(links.links.len(), 2);
        assert_eq!(links.depth, 2);
    }

    #[test]
    fn test_fetched_page_equality_arc_body() {
        let body = std::sync::Arc::new(vec![1, 2, 3]);

        let p1 = FetchedPage {
            task: sample_task(),
            status_code: 200,
            content_type: Some("text/plain".into()),
            fetch_time: 1,
            body: body.clone(),
        };

        let p2 = FetchedPage {
            task: sample_task(),
            status_code: 200,
            content_type: Some("text/plain".into()),
            fetch_time: 1,
            body,
        };

        assert_eq!(p1, p2);
    }
}
