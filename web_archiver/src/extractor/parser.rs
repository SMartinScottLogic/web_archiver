use std::collections::HashSet;

use anyhow::Result;
use common::markdown::html_to_markdown;
use common::url::{canonicalize_url, resolve_relative_link};
use lazy_static::lazy_static;
use map_macro::hash_map;
use scraper::{Html, Selector};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, trace};

use crate::extractor::router::Steve;
use crate::extractor::{DiscoveredLink, DiscoveredLinks, FetchedPage};

pub async fn extractor_loop(
    mut rx: Receiver<FetchedPage>,
    tx_storage: Sender<Steve>,
    tx_frontier: Sender<DiscoveredLinks>,
) {
    while let Some(fetched) = rx.recv().await {
        debug!("Extractor received page: {}", fetched.task.url);
        if let Ok((page, links)) = extract_page(fetched).await {
            debug!(
                "Extractor send extracted page to storage ({}/{})",
                tx_storage.capacity(),
                tx_storage.max_capacity()
            );

            let _ = tx_storage.send(page).await;
            debug!(
                "Extractor send discovered links to frontier ({}/{})",
                tx_frontier.capacity(),
                tx_frontier.max_capacity()
            );
            let _ = tx_frontier.send(links).await;
        }
        debug!("Extractor waiting for fetched page");
    }
}

lazy_static! {
    static ref IGNORE_DOCUMENT_METADATA: HashSet<&'static str> = {
        let mut meta = HashSet::new();
        meta.insert("viewport");
        meta.insert("twitter:title");
        meta.insert("twitter:description");
        meta.insert("twitter:card");
        meta.insert("twitter:site");
        meta.insert("twitter:image");
        meta
    };
}

async fn extract_page(fetched: FetchedPage) -> Result<(Steve, DiscoveredLinks)> {
    let html = String::from_utf8_lossy(&fetched.body);
    let document = Html::parse_document(&html);

    // Extract document metadata
    let selector = Selector::parse("meta[name]").unwrap();
    let mut meta = vec![];
    for element in document.select(&selector) {
        if let Some(name) = element.value().attr("name")
            && let Some(content) = element.value().attr("content")
            && !IGNORE_DOCUMENT_METADATA.contains(name)
        {
            meta.push(hash_map!["name".to_string() => name.to_string(), "content".to_string() => content.to_string()]);
            trace!(name, content, url = fetched.task.url, "metadata");
        }
    }
    // Extract <a href> links
    let selector = Selector::parse("a[href]").unwrap();
    let mut links = HashSet::new();
    for element in document.select(&selector) {
        if let Some(href) = element.value().attr("href")
            && let Some(resolved) = resolve_relative_link(&fetched.task.url, href)
            && let Some(canon) = canonicalize_url(&resolved)
        {
            links.insert(canon);
        }
    }

    debug!(
        "Extractor: {} links discovered from {}",
        links.len(),
        fetched.task.url
    );

    // Extract page text as markdown
    let html = String::from_utf8_lossy(&fetched.body);
    let markdown = html_to_markdown(&html, &fetched.task.url);
    let next_depth = fetched.task.depth + 1;
    let this_url_id = fetched.task.url_id;

    let discovered_links = DiscoveredLinks {
        parent_url_id: this_url_id,
        links: links
            .iter()
            .map(|url| {
                let priority = get_priority(&fetched.task.url, url);
                DiscoveredLink { url: url.to_owned(), priority }
            })
            .collect(),
        depth:next_depth,
    };

    let historical_snapshot = Steve { task: fetched.task, content: markdown, fetch_time: chrono::Utc::now().timestamp(), links };

    // let historical_snapshot = HistoricalSnapshot {
    //     content_markdown: vec![HistoricalContent {
    //         content: common::historical::HistoricalContentType::Literal(markdown),
    //         page: u32::MAX,
    //     }],
    //     links,
    //     metadata: Some(PageMetadata {
    //         status_code: 200,
    //         content_type: None,
    //         fetch_time: chrono::Utc::now().timestamp() as u64,
    //         title: document
    //             .select(&Selector::parse("title").unwrap())
    //             .next()
    //             .map(|e| e.text().collect::<String>()),
    //         document_metadata: Some(meta),
    //     }),
    // };

    // let extracted_page = ExtractedPage {
    //     task: fetched.task.clone(),
    //     content_markdown: Some(markdown),
    //     links: links.clone(),
    //     metadata: Some(PageMetadata {
    //         status_code: 200,
    //         content_type: None,
    //         fetch_time: chrono::Utc::now().timestamp() as u64,
    //         title: document
    //             .select(&Selector::parse("title").unwrap())
    //             .next()
    //             .map(|e| e.text().collect::<String>()),
    //         document_metadata: Some(meta),
    //     }),
    // };

    debug!("Extractor done");

    Ok((historical_snapshot, discovered_links))
}

// Raise priority if page of same article
fn get_priority(source: &str, target: &str) -> i32 {
    if common::url::remove_pagination_params(source) == common::url::remove_pagination_params(target) {
        10
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::types::FetchTask;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_extract_page_basic() {
        let html = b"<html><body><a href='https://foo.com/bar'>link</a>Text</body></html>".to_vec();
        let fetched = FetchedPage {
            task: FetchTask {
                article_id: 0,
                url_id: 1,
                url: "https://foo.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            status_code: 200,
            content_type: Some("text/html".to_string()),
            fetch_time: 0,
            body: Arc::new(html),
        };
        let (extracted, discovered) = extract_page(fetched).await.unwrap();
        assert_eq!("Text", extracted.content);
        assert_eq!(discovered.links.len(), 1);
        assert!(discovered.links[0].url.contains("foo.com/bar"));
    }

    #[tokio::test]
    async fn test_extract_page_metadata() {
        let html = b"<html><head><meta name='test' content='test content' /></head><body><a href='https://foo.com/bar'>link</a>Text</body></html>".to_vec();
        let fetched = FetchedPage {
            task: FetchTask {
                article_id: 0,
                url_id: 1,
                url: "https://foo.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            status_code: 200,
            content_type: Some("text/html".to_string()),
            fetch_time: 0,
            body: Arc::new(html),
        };
        let (extracted, discovered) = extract_page(fetched).await.unwrap();
        assert_eq!("Text", extracted.content);
        assert_eq!(discovered.links.len(), 1);
        assert!(discovered.links[0].url.contains("foo.com/bar"));
        // TODO Document metadata
        // let document_metadata = extracted.metadata.unwrap().document_metadata.unwrap();
        // assert_eq!(document_metadata.len(), 1);
        // assert_eq!(
        //     document_metadata[0].get("name"),
        //     Some("test".to_string()).as_ref()
        // );
        // assert_eq!(
        //     document_metadata[0].get("content"),
        //     Some("test content".to_string()).as_ref()
        // );
    }

    #[tokio::test]
    async fn test_extractor_loop_sends_outputs() {
        use std::sync::Arc;
        use tokio::sync::mpsc;

        let (tx_fetched, rx_fetched) = mpsc::channel(1);
        let (tx_storage, mut rx_storage) = mpsc::channel(1);
        let (tx_frontier, mut rx_frontier) = mpsc::channel(1);

        // Send a test FetchedPage
        let html = b"<html><body><a href='https://foo.com/bar'>link</a>Text</body></html>".to_vec();
        let fetched = FetchedPage {
            task: FetchTask {
                article_id: 0,
                url_id: 1,
                url: "https://foo.com".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            status_code: 200,
            content_type: Some("text/html".to_string()),
            fetch_time: 0,
            body: Arc::new(html),
        };
        tx_fetched.send(fetched).await.unwrap();
        drop(tx_fetched); // Close channel

        extractor_loop(rx_fetched, tx_storage.clone(), tx_frontier.clone()).await;

        // Check that outputs were sent
        let extracted: Steve = rx_storage.try_recv().unwrap();
        let discovered: DiscoveredLinks = rx_frontier.try_recv().unwrap();
        assert_eq!("Text", extracted.content);
        assert_eq!(discovered.links.len(), 1);
        assert!(discovered.links[0].url.contains("foo.com/bar"));
    }
}
