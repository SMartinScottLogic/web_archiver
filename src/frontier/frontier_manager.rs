use crate::types::messages::{DiscoveredLinks, FetchTask};
use crate::util::canonicalize_url;
use std::collections::{HashSet, VecDeque};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error, trace};

/// Minimal Week 1 frontier manager.
/// Maintains in-memory queue and seen cache, sends FetchTasks to workers.
pub struct FrontierManager {
    queue: VecDeque<FetchTask>,
    seen: HashSet<String>,
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
    ) -> Self {
        let mut queue = VecDeque::new();
        let mut seen = HashSet::new();

        for (i, url) in seed_urls.into_iter().enumerate() {
            if let Some(canonical) = canonicalize_url(&url) {
                seen.insert(canonical.clone());
                queue.push_back(FetchTask {
                    url_id: i as i64,
                    url: canonical,
                    depth: 0,
                    priority: 0,
                    discovered_from: None,
                });
            }
        }

        Self {
            queue,
            seen,
            tx_fetch,
            rx_links,
            noop_delay_millis,
            allowed_domains,
        }
    }

    pub async fn run(mut self) {
        loop {
            // --- 1. Dispatch tasks if queue not empty ---
            debug!("dispatching tasks ({})", self.queue.len());
            // while let Some(task) = self.queue.pop_front() {
            //     debug!("dispatch task {:?} ({}/{})", &task, self.tx_fetch.capacity(), self.tx_fetch.max_capacity());
            //     if (self.tx_fetch.send(task).await).is_err() {
            //         error!("Worker channel closed, frontier stopping");
            //         return;
            //     }
            //     debug!("queue capacity: {}", self.queue.len());
            // }
            if self.tx_fetch.capacity() > 0
                && let Some(task) = self.queue.pop_front()
            {
                debug!(
                    "dispatch task {:?} ({}/{})",
                    &task,
                    self.tx_fetch.capacity(),
                    self.tx_fetch.max_capacity()
                );
                if (self.tx_fetch.send(task).await).is_err() {
                    error!("Worker channel closed, frontier stopping");
                    return;
                }
                debug!("queue capacity: {}", self.queue.len());
            }
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
        for link in msg.links {
            if self.seen.contains(&link) {
                continue;
            }
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
            self.seen.insert(link.clone());

            let next_id = self.seen.len() as i64; // simple ID
            let task = FetchTask {
                url_id: next_id,
                url: link,
                depth: msg.depth,
                priority: 0,
                discovered_from: Some(msg.parent_url_id),
            };
            self.queue.push_back(task);
        }
    }
}
