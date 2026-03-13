use crate::types::messages::{DiscoveredLinks, ExtractedPage, FetchedPage, PageMetadata};
use crate::util::html_to_markdown;
use crate::util::{canonicalize_url, resolve_relative_link};
use anyhow::Result;
use scraper::{Html, Selector};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::debug;

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

async fn extract_page(fetched: FetchedPage) -> Result<(ExtractedPage, DiscoveredLinks)> {
    let html = String::from_utf8_lossy(&fetched.body);
    let document = Html::parse_document(&html);

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
