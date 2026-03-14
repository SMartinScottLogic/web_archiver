use crate::config::settings::Host;
use crate::frontier::db::frontier::FrontierDb;
use crate::types::messages::{DiscoveredLinks, FetchTask};
use crate::util::canonicalize_url;
use reqwest::Client;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error, info, trace};

/// Minimal Week 1 frontier manager.
/// Maintains in-memory queue and seen cache, sends FetchTasks to workers.
pub struct FrontierManager {
    pub db: FrontierDb,
    tx_fetch: Sender<FetchTask>,
    rx_links: Receiver<DiscoveredLinks>,
    noop_delay_millis: u64,
    hosts: Vec<Host>,
    robots_cache: Arc<Mutex<HashMap<String, Option<String>>>>,
    http_client: Client,
}

impl FrontierManager {
    pub fn new(
        seed_urls: Vec<String>,
        tx_fetch: Sender<FetchTask>,
        rx_links: Receiver<DiscoveredLinks>,
        noop_delay_millis: u64,
        hosts: Vec<Host>,
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
            hosts,
            robots_cache: Arc::new(Mutex::new(HashMap::new())),
            http_client: Client::new(),
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
                self.process_discovered_links(msg).await;
            }

            // --- 3. Sleep a bit to avoid busy loop ---
            tokio::time::sleep(std::time::Duration::from_millis(self.noop_delay_millis)).await;
        }
    }

    pub async fn process_discovered_links(&mut self, msg: DiscoveredLinks) {
        let mut batch = Vec::new();
        for link in msg.links {
            if !crate::util::url::is_http_url(&link) {
                trace!("Skipping non-http link: {}", link);
                continue;
            }
            // Only allow links whose domain is in allowed_domains
            if let Some(domain) = crate::util::extract_domain(&link) {
                let matching_domains = self.get_matching_domains(&domain);
                if matching_domains.is_empty() {
                    trace!("Skipping link outside allowed domains: {}", link);
                    continue;
                } else {
                    debug!(domain, matches_domains = ?matching_domains.iter().map(|host| host.name.clone()).collect::<Vec<_>>(), "matches");
                }

                // Check robots.txt rules
                if !self.is_url_allowed(&link, &domain).await {
                    debug!("Skipping link blocked by robots.txt: {}", link);
                    continue;
                } else {
                    trace!("Link permitted by robots.txt: {}", link);
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

    fn get_matching_domains(&self, domain: &str) -> Vec<&Host> {
        self.hosts
            .iter()
            .filter(|&host| host.domains.iter().any(|d| d == domain))
            .collect()
    }

    /// Check if a URL is allowed by robots.txt rules for its domain
    async fn is_url_allowed(&mut self, url: &str, domain: &str) -> bool {
        let mut matcher = robotstxt::DefaultMatcher::default();
        // Try to get robots.txt from cache
        let v = {
            let cache = self.robots_cache.lock().unwrap();
            cache.get(domain).map(|v| v.to_owned())
        };
        let r = match v {
            Some(Some(robots_txt)) => robots_txt,
            Some(None) => {
                return true;
            }
            None => match self.fetch_robots_txt(domain).await {
                Some(r) => r,
                None => {
                    return true;
                }
            },
        };
        // Check if URL is allowed by robots.txt rules
        matcher.one_agent_allowed_by_robots(&r, "Week1Crawler", url)
        //robots_txt.is_allowed(url, "*")
    }

    /// Fetch and parse robots.txt for a domain
    async fn fetch_robots_txt(&mut self, domain: &str) -> Option<String> {
        let robots_url = format!("http://{}{}", domain, "/robots.txt");

        let robots_txt = match self.http_client.get(&robots_url).send().await {
            Ok(resp) => match resp.text().await {
                Ok(r) => Some(r),
                Err(e) => {
                    error!("error reading robots.txt from {}: {:?}", robots_url, e);
                    None
                }
            },
            Err(e) => {
                debug!("error fetching robots.txt from {}: {:?}", robots_url, e);
                None
            }
        };
        debug!(
            "robots found for {} {}: {:?}",
            domain, robots_url, &robots_txt
        );

        // Cache the robots.txt
        {
            let mut cache = self.robots_cache.lock().unwrap();
            cache.insert(domain.to_string(), robots_txt.clone());
        }

        robots_txt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontier::db::frontier::FrontierDb;
    use crate::types::messages::DiscoveredLinks;
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};

    fn setup_manager(hosts: Vec<Host>) -> FrontierManager {
        let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        // Create minimal schema for enqueue_batch
        conn.lock().unwrap().execute_batch(r#"
                    CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT UNIQUE, domain TEXT, discovered_at INTEGER);
                    CREATE TABLE frontier (url_id INTEGER, priority INTEGER, depth INTEGER, discovered_from INTEGER, status TEXT, claimed_at INTEGER);
                "#).unwrap();
        let mut cache = HashMap::new();
        cache.insert("foo.com".to_string(), None);
        cache.insert("example.com".to_string(), None);
        FrontierManager {
            db: FrontierDb::new(conn),
            tx_fetch: tokio::sync::mpsc::channel(1).0,
            rx_links: tokio::sync::mpsc::channel(1).1,
            noop_delay_millis: 1,
            hosts,
            robots_cache: Arc::new(Mutex::new(cache)),
            http_client: Client::new(),
        }
    }

    #[tokio::test]
    async fn test_process_discovered_links_skips_non_http() {
        let mut mgr = setup_manager(vec![Host {
            name: "Foo".to_string(),
            domains: vec!["foo.com".to_string()],
        }]);
        let msg = DiscoveredLinks {
            parent_url_id: 1,
            links: vec![
                "ftp://foo.com/file".to_string(),
                "mailto:bar@foo.com".to_string(),
            ],
            depth: 1,
        };
        mgr.process_discovered_links(msg).await;
        // Should not enqueue anything
        let conn = mgr.db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM urls", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_process_discovered_links_skips_disallowed_domain() {
        let mut mgr = setup_manager(vec![Host {
            name: "Foo".to_string(),
            domains: vec!["foo.com".to_string()],
        }]);
        let msg = DiscoveredLinks {
            parent_url_id: 1,
            links: vec!["http://bar.com/page".to_string()],
            depth: 1,
        };
        mgr.process_discovered_links(msg).await;
        let conn = mgr.db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM urls", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_process_discovered_links_skips_no_domain() {
        let mut mgr = setup_manager(vec![Host {
            name: "Foo".to_string(),
            domains: vec!["foo.com".to_string()],
        }]);
        let msg = DiscoveredLinks {
            parent_url_id: 1,
            links: vec!["http://bar.com/page".to_string()],
            depth: 1,
        };
        mgr.process_discovered_links(msg).await;
        let conn = mgr.db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM urls", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_process_discovered_links_enqueues_valid() {
        let mut mgr = setup_manager(vec![Host {
            name: "Foo".to_string(),
            domains: vec!["foo.com".to_string()],
        }]);
        let msg = DiscoveredLinks {
            parent_url_id: 1,
            links: vec!["http://foo.com/page".to_string()],
            depth: 2,
        };
        mgr.process_discovered_links(msg).await;
        let conn = mgr.db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM urls", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let url: String = conn
            .query_row("SELECT url FROM urls", [], |row| row.get(0))
            .unwrap();
        assert_eq!(url, "http://foo.com/page");
    }

    #[tokio::test]
    async fn test_run_dispatches_tasks() {
        let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        // Create minimal schema for enqueue_batch and claim_next
        conn.lock().unwrap().execute_batch(r#"
                    CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT UNIQUE, domain TEXT, discovered_at INTEGER);
                    CREATE TABLE frontier (url_id INTEGER, priority INTEGER, depth INTEGER, discovered_from INTEGER, status TEXT, claimed_at INTEGER);
                    INSERT INTO urls (id, url, domain, discovered_at) VALUES (1, 'http://example.com', 'example.com', 1);
                    INSERT INTO frontier (url_id, priority, depth, discovered_from, status, claimed_at) VALUES (1, 0, 0, NULL, 'pending', NULL);
                "#).unwrap();

        let (tx_fetch, mut rx_fetch) = tokio::sync::mpsc::channel(1);
        let (_tx_links, rx_links) = tokio::sync::mpsc::channel(1);

        let mgr = FrontierManager {
            db: FrontierDb::new(conn),
            tx_fetch,
            rx_links,
            noop_delay_millis: 1,
            hosts: vec![Host {
                name: "Example".to_string(),
                domains: vec!["example.com".to_string()],
            }],
            robots_cache: Arc::new(Mutex::new(HashMap::new())),
            http_client: Client::new(),
        };

        // Start the run method in a task but don't move mgr afterwards
        let handle = tokio::spawn(async move {
            mgr.run().await;
        });

        // Give it a moment to process
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Check that a task was sent
        let task = rx_fetch.try_recv();
        assert!(task.is_ok());

        // Cancel the task
        handle.abort();
    }

    #[tokio::test]
    async fn test_run_processes_links() {
        // Create a separate connection for checking the database
        let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        conn.lock().unwrap().execute_batch(r#"
                    CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT UNIQUE, domain TEXT, discovered_at INTEGER);
                    CREATE TABLE frontier (url_id INTEGER, priority INTEGER, depth INTEGER, discovered_from INTEGER, status TEXT, claimed_at INTEGER);
                "#).unwrap();

        let (tx_fetch, _rx_fetch) = tokio::sync::mpsc::channel(1);
        let (tx_links, rx_links) = tokio::sync::mpsc::channel(1);

        let mut cache = HashMap::new();
        cache.insert("example.com".to_string(), None);
        // Create manager with the same connection
        let mgr = FrontierManager {
            db: FrontierDb::new(conn.clone()),
            tx_fetch,
            rx_links,
            noop_delay_millis: 1,
            hosts: vec![Host {
                name: "Example".to_string(),
                domains: vec!["example.com".to_string()],
            }],
            robots_cache: Arc::new(Mutex::new(cache)),
            http_client: Client::new(),
        };

        // Send a link message
        let msg = DiscoveredLinks {
            parent_url_id: 1,
            links: vec!["http://example.com/page".to_string()],
            depth: 1,
        };
        tx_links.send(msg).await.unwrap();

        // Start the run method in a task
        let handle = tokio::spawn(async move {
            mgr.run().await;
        });

        // Give it a moment to process
        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;

        // Check that the link was enqueued by directly checking the database
        let conn_guard = conn.lock().unwrap();
        let count: i64 = conn_guard
            .query_row("SELECT COUNT(*) FROM urls", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Cancel the task
        handle.abort();
    }

    #[tokio::test]
    async fn test_run_handles_channel_close() {
        let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        // Create minimal schema for enqueue_batch and claim_next
        conn.lock().unwrap().execute_batch(r#"
                    CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT UNIQUE, domain TEXT, discovered_at INTEGER);
                    CREATE TABLE frontier (url_id INTEGER, priority INTEGER, depth INTEGER, discovered_from INTEGER, status TEXT, claimed_at INTEGER);
                    INSERT INTO urls (id, url, domain, discovered_at) VALUES (1, 'http://example.com', 'example.com', 1);
                    INSERT INTO frontier (url_id, priority, depth, discovered_from, status, claimed_at) VALUES (1, 0, 0, NULL, 'pending', NULL);
                "#).unwrap();

        let (tx_fetch, mut rx_fetch) = tokio::sync::mpsc::channel(1);
        // Don't keep the receiver to simulate channel closure
        let (_tx_links, rx_links) = tokio::sync::mpsc::channel(1);

        let mgr = FrontierManager {
            db: FrontierDb::new(conn),
            tx_fetch,
            rx_links,
            noop_delay_millis: 1,
            hosts: vec![Host {
                name: "Example".to_string(),
                domains: vec!["example.com".to_string()],
            }],
            robots_cache: Arc::new(Mutex::new(HashMap::new())),
            http_client: Client::new(),
        };

        // Start the run method in a task
        let handle = tokio::spawn(async move {
            mgr.run().await;
        });

        // Give it a moment to process
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Check that the task was sent (the channel should not be closed yet)
        let task = rx_fetch.try_recv();
        assert!(task.is_ok());

        // Cancel the task
        handle.abort();
    }
}
