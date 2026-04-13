//! Unit tests for the FrontierManager (integration with DB and link processing)

use common::settings::Host;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use web_archiver::{extractor::{DiscoveredLink, DiscoveredLinks}, frontier::frontier_manager::FrontierManager};

fn setup_manager(seed_urls: Vec<String>, hosts: Vec<Host>) -> FrontierManager {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE articles (
            id INTEGER PRIMARY KEY,
            url TEXT NOT NULL UNIQUE
        );
        CREATE TABLE urls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            url TEXT UNIQUE NOT NULL,
            article_id INTEGER NOT NULL,
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
        CREATE UNIQUE INDEX idx_frontier_url_id ON frontier(url_id);
    "#,
    )
    .unwrap();
    let (tx_fetch, _rx_fetch) = mpsc::channel(10);
    let (_tx_links, rx_links) = mpsc::channel(10);
    FrontierManager::new(
        "user_agent".to_string(),
        seed_urls,
        tx_fetch,
        rx_links,
        1,
        hosts,
        Arc::new(Mutex::new(conn)),
    )
}

#[tokio::test]
async fn test_seed_batch_insertion_and_claim() {
    let mgr = setup_manager(
        vec!["http://foo.com".to_string(), "http://bar.com".to_string()],
        vec![
            Host {
                name: "Foo".to_string(),
                domains: vec!["foo.com".to_string()],
                pages: Default::default(),
            },
            Host {
                name: "Bar".to_string(),
                domains: vec!["bar.com".to_string()],
                pages: Default::default(),
            },
        ],
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
    let mut mgr = setup_manager(
        vec![],
        vec![Host {
            name: "Foo".to_string(),
            domains: vec!["foo.com".to_string()],
            pages: Default::default(),
        }],
    );
    let msg = DiscoveredLinks {
        links: vec![
            DiscoveredLink { url: "http://foo.com/page1".to_string(), priority: 0 },
            DiscoveredLink { url: "http://bar.com/page2".to_string(), priority: 0 }, // not allowed
            DiscoveredLink { url: "ftp://foo.com/file".to_string(), priority: 0 },   // not http
        ],
        depth: 1,
        parent_url_id: 1,
    };
    mgr.process_discovered_links(msg).await;
    // Only http://foo.com/page1 should be enqueued
    let t = mgr.db.claim_next().unwrap().unwrap();
    assert_eq!(t.url, "http://foo.com/page1");
    assert_eq!(t.depth, 1);
    assert_eq!(t.discovered_from, Some(1));
    // No more tasks
    assert!(mgr.db.claim_next().unwrap().is_none());
}
