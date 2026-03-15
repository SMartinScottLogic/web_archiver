use std::collections::HashSet;

use crate::types::messages::{DiscoveredLinks, ExtractedPage, FetchedPage, PageMetadata};
use crate::util::html_to_markdown;
use crate::util::{canonicalize_url, resolve_relative_link};
use anyhow::Result;
use lazy_static::lazy_static;
use map_macro::hash_map;
use scraper::{Html, Selector};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, trace};

pub async fn extractor_loop(
    mut rx: Receiver<FetchedPage>,
    tx_storage: Sender<ExtractedPage>,
    tx_frontier: Sender<DiscoveredLinks>,
) {
    while let Some(fetched) = rx.recv().await {
        debug!("Extractor received page: {}", fetched.task.url);
        if let Ok(extracted) = extract_page(fetched).await {
            debug!(
                "Extractor send extracted page to storage ({}/{})",
                tx_storage.capacity(),
                tx_storage.max_capacity()
            );

            let _ = tx_storage.send(extracted.0).await;
            debug!(
                "Extractor send discovered links to frontier ({}/{})",
                tx_frontier.capacity(),
                tx_frontier.max_capacity()
            );
            let _ = tx_frontier.send(extracted.1).await;
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

async fn extract_page(fetched: FetchedPage) -> Result<(ExtractedPage, DiscoveredLinks)> {
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
    let mut links = vec![];
    for element in document.select(&selector) {
        if let Some(href) = element.value().attr("href")
            && let Some(resolved) = resolve_relative_link(&fetched.task.url, href)
            && let Some(canon) = canonicalize_url(&resolved)
        {
            links.push(canon);
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

    let extracted_page = ExtractedPage {
        task: fetched.task.clone(),
        content_markdown: Some(markdown),
        links: links.clone(),
        metadata: PageMetadata {
            status_code: 200,
            content_type: None,
            fetch_time: chrono::Utc::now().timestamp() as u64,
            title: document
                .select(&Selector::parse("title").unwrap())
                .next()
                .map(|e| e.text().collect::<String>()),
            document_metadata: meta,
        },
    };

    let discovered_links = DiscoveredLinks {
        parent_url_id: fetched.task.url_id,
        links,
        depth: fetched.task.depth + 1,
    };

    debug!("Extractor done");

    Ok((extracted_page, discovered_links))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::messages::FetchTask;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_extract_page_basic() {
        let html = b"<html><body><a href='https://foo.com/bar'>link</a></body></html>".to_vec();
        let fetched = FetchedPage {
            task: FetchTask {
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
        assert!(extracted.content_markdown.is_some());
        assert_eq!(discovered.links.len(), 1);
        assert!(discovered.links[0].contains("foo.com/bar"));
    }

    #[tokio::test]
    async fn test_extract_page_metadata() {
        let html = b"<html><head><meta name='test' content='test content' /></head><body><a href='https://foo.com/bar'>link</a></body></html>".to_vec();
        let fetched = FetchedPage {
            task: FetchTask {
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
        assert!(extracted.content_markdown.is_some());
        assert_eq!(discovered.links.len(), 1);
        assert!(discovered.links[0].contains("foo.com/bar"));
        assert_eq!(extracted.metadata.document_metadata.len(), 1);
        assert_eq!(
            extracted.metadata.document_metadata[0].get("name"),
            Some("test".to_string()).as_ref()
        );
        assert_eq!(
            extracted.metadata.document_metadata[0].get("content"),
            Some("test content".to_string()).as_ref()
        );
    }

    #[tokio::test]
    async fn test_extractor_loop_sends_outputs() {
        use crate::types::messages::{DiscoveredLinks, ExtractedPage, FetchedPage};
        use std::sync::Arc;
        use tokio::sync::mpsc;

        let (tx_fetched, rx_fetched) = mpsc::channel(1);
        let (tx_storage, mut rx_storage) = mpsc::channel(1);
        let (tx_frontier, mut rx_frontier) = mpsc::channel(1);

        // Send a test FetchedPage
        let html = b"<html><body><a href='https://foo.com/bar'>link</a></body></html>".to_vec();
        let fetched = FetchedPage {
            task: FetchTask {
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
        let extracted: ExtractedPage = rx_storage.try_recv().unwrap();
        let discovered: DiscoveredLinks = rx_frontier.try_recv().unwrap();
        assert!(extracted.content_markdown.is_some());
        assert_eq!(discovered.links.len(), 1);
        assert!(discovered.links[0].contains("foo.com/bar"));
    }
}
