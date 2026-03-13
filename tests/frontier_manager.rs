//! Unit tests for the FrontierManager (integration with DB and link processing)

use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use web_archiver::frontier::frontier_manager::FrontierManager;
use web_archiver::types::messages::DiscoveredLinks;

fn setup_manager(seed_urls: Vec<String>, allowed_domains: Vec<String>) -> FrontierManager {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE urls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            url TEXT UNIQUE NOT NULL,
            domain TEXT,
            discovered_at INTEGER
        );
        CREATE TABLE frontier (
            url_id INTEGER,
            priority INTEGER,
            depth INTEGER,
            discovered_from INTEGER,
            status TEXT,
            claimed_at INTEGER,
            FOREIGN KEY(url_id) REFERENCES urls(id),
            UNIQUE(url_id)
        );
    "#,
    )
    .unwrap();
    let (tx_fetch, _rx_fetch) = mpsc::channel(10);
    let (_tx_links, rx_links) = mpsc::channel(10);
    FrontierManager::new(
        seed_urls,
        tx_fetch,
        rx_links,
        1,
        allowed_domains,
        Arc::new(Mutex::new(conn)),
    )
}

#[tokio::test]
async fn test_seed_batch_insertion_and_claim() {
    let mgr = setup_manager(
        vec!["http://foo.com".to_string(), "http://bar.com".to_string()],
        vec!["foo.com".to_string(), "bar.com".to_string()],
    );
    // Should be able to claim both seeds
    let t1 = mgr.db.claim_next().unwrap().unwrap();
    let t2 = mgr.db.claim_next().unwrap().unwrap();
    dbg!(t1.url.clone(), t2.url.clone());
    assert!(t1.url == "http://foo.com/" || t1.url == "http://bar.com/");
    assert!(t2.url == "http://foo.com/" || t2.url == "http://bar.com/");
    assert_ne!(t1.url, t2.url);
}

#[tokio::test]
async fn test_process_discovered_links_batching_and_filtering() {
    let mut mgr = setup_manager(vec![], vec!["foo.com".to_string()]);
    let msg = DiscoveredLinks {
        links: vec![
            "http://foo.com/page1".to_string(),
            "http://bar.com/page2".to_string(), // not allowed
            "ftp://foo.com/file".to_string(),   // not http
        ],
        depth: 1,
        parent_url_id: 1,
    };
    mgr.process_discovered_links(msg);
    // Only http://foo.com/page1 should be enqueued
    let t = mgr.db.claim_next().unwrap().unwrap();
    assert_eq!(t.url, "http://foo.com/page1");
    assert_eq!(t.depth, 1);
    assert_eq!(t.discovered_from, Some(1));
    // No more tasks
    assert!(mgr.db.claim_next().unwrap().is_none());
}
