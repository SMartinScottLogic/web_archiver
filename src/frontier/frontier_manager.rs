use crate::frontier::db::frontier::FrontierDb;
use crate::types::messages::{DiscoveredLinks, FetchTask};
use crate::util::canonicalize_url;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error, info, trace};

/// Minimal Week 1 frontier manager.
/// Maintains in-memory queue and seen cache, sends FetchTasks to workers.
pub struct FrontierManager {
    db: FrontierDb,
    tx_fetch: Sender<FetchTask>,
    rx_links: Receiver<DiscoveredLinks>,
    noop_delay_millis: u64,
    allowed_domains: Vec<String>,
}

impl FrontierManager {
    pub fn new(
        seed_urls: Vec<String>,
        tx_fetch: Sender<FetchTask>,
        rx_links: Receiver<DiscoveredLinks>,
        noop_delay_millis: u64,
        allowed_domains: Vec<String>,
        db_conn: Arc<Mutex<Connection>>,
    ) -> Self {
        let db = FrontierDb::new(db_conn.clone());
        // Batch insert seed URLs into DB
        let mut seeds = Vec::new();
        for url in seed_urls {
            if let Some(canonical) = canonicalize_url(&url) {
                seeds.push(FetchTask {
                    url_id: 0, // Will be set by DB
                    url: canonical,
                    depth: 0,
                    priority: 0,
                    discovered_from: None,
                });
            }
        }
        if !seeds.is_empty() {
            let _ = db.enqueue_batch(&seeds);
        }
        Self {
            db,
            tx_fetch,
            rx_links,
            noop_delay_millis,
            allowed_domains,
        }
    }

    pub async fn run(mut self) {
        loop {
            // --- 1. Dispatch tasks from DB queue ---
            if self.tx_fetch.capacity() > 0
                && let Ok(Some(task)) = self.db.claim_next()
            {
                debug!("dispatch task {:?}", &task);
                if (self.tx_fetch.send(task).await).is_err() {
                    error!("Worker channel closed, frontier stopping");
                    return;
                }
            }

            // --- LOG: Show fetched and total pages ---
            let fetched = self.db.count_fetched().unwrap_or(0);
            let pending = self.db.count_pending().unwrap_or(0);
            let total = fetched + pending;
            info!(fetched, total, "Frontier progress");

            debug!(
                "receiving links ({}/{})",
                self.rx_links.capacity(),
                self.rx_links.max_capacity()
            );
            // --- 2. Receive discovered links ---
            while let Ok(msg) = self.rx_links.try_recv() {
                debug!("receive {} links", msg.links.len());
                self.process_discovered_links(msg);
            }

            // --- 3. Sleep a bit to avoid busy loop ---
            tokio::time::sleep(std::time::Duration::from_millis(self.noop_delay_millis)).await;
        }
    }

    fn process_discovered_links(&mut self, msg: DiscoveredLinks) {
        let mut batch = Vec::new();
        for link in msg.links {
            if !crate::util::url::is_http_url(&link) {
                trace!("Skipping non-http link: {}", link);
                continue;
            }
            // Only allow links whose domain is in allowed_domains
            if let Some(domain) = crate::util::extract_domain(&link) {
                if !self.allowed_domains.iter().any(|d| d == &domain) {
                    trace!("Skipping link outside allowed domains: {}", link);
                    continue;
                }
            } else {
                trace!("Skipping link with no domain: {}", link);
                continue;
            }
            batch.push(FetchTask {
                url_id: 0, // Will be set by DB
                url: link,
                depth: msg.depth,
                priority: 0,
                discovered_from: Some(msg.parent_url_id),
            });
        }
        if !batch.is_empty() {
            let _ = self.db.enqueue_batch(&batch);
        }
    }
}
