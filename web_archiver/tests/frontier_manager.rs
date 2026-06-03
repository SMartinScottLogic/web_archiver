//! Unit tests for the FrontierManager (integration with DB and link processing)

use common::{
    settings::Host,
    types::{FetchTask, Priority},
    url::canonicalize_url,
};
use rusqlite::{Connection, types::Value};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{error, info};
use web_archiver::{
    extractor::{DiscoveredLink, DiscoveredLinks},
    frontier::frontier_manager::FrontierManager,
    settings::{CONFIG, Config},
};

fn dump_table(manager: &FrontierManager, table: &str) -> anyhow::Result<()> {
    let conn = manager.db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(&format!("SELECT * FROM {table}"))
        .inspect_err(|e| error!("Failed to prepare query: {:?}", e))?;
    let v = stmt
        .query_map([], |row| {
            let mut idx = 0;
            let mut values = Vec::new();
            loop {
                let value = row.get::<usize, Value>(idx);
                if value.is_err() {
                    break Ok(values);
                }
                let value = value.unwrap();
                values.push(value);
                idx += 1;
            }
        })
        .inspect_err(|e| error!("Failed to get next url: {:?}", e))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .inspect_err(|e| error!("Failed to get next url: {:?}", e))?;
    info!(?v, table, "rows");
    Ok(())
}

fn setup_test_logging() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
}
fn setup_test_config() {
    CONFIG.get_or_init(|| Config {
        archive_dir: "test_archive".into(),
        archive_time: 0,
        hosts: vec![
            Host {
                name: "Foo".to_string(),
                domains: vec!["foo.com".to_string()],
                pages: Default::default(),
                use_playwright: false,
                ignore_robots: false,
                max_depth: None,
            },
            Host {
                name: "Bar".to_string(),
                domains: vec!["bar.com".to_string()],
                pages: Default::default(),
                use_playwright: false,
                ignore_robots: false,
                max_depth: None,
            },
        ],
        mailboxes: Vec::new(),
        workers: 0,
        seed_urls: Vec::new(),
        noop_delay_millis: 0,
        user_agent: "test".into(),
        db: "test.db".into(),
        reset: false,
        ..Default::default()
    });
}

/// Batch insert seed URLs into DB
fn add_seeds(manager: &FrontierManager, seed_urls: &[&str]) {
    let mut seeds = Vec::new();
    for url in seed_urls {
        if let Some(canonical) = canonicalize_url(url) {
            // TODO Use hosts to set this correctly
            let use_playwright = false;
            seeds.push(FetchTask {
                article_id: 0, // Will be set by DB
                url_id: 0,     // Will be set by DB
                url: canonical,
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
                use_playwright,
            });
        }
    }
    if !seeds.is_empty() {
        let _ = manager
            .db
            .enqueue_batch(&seeds, true)
            .inspect_err(|e| error!("enqueue seeds failed {:?}", e));
    }
}

fn setup_manager() -> FrontierManager {
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
            discovered_at INTEGER,
            use_playwright INTEGER NOT NULL DEFAULT 0
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
    let mut manager = FrontierManager::new(
        //"user_agent".to_string(),
        tx_fetch,
        rx_links,
        //1,
        //hosts,
        Arc::new(Mutex::new(conn)),
    );
    manager.add_seeds();
    //manager.add_seeds(&seed_urls);
    manager
}

#[tokio::test]
async fn test_seed_batch_insertion_and_claim() {
    setup_test_logging();
    setup_test_config();

    let mgr = setup_manager();
    add_seeds(&mgr, &["http://foo.com", "http://bar.com"]);
    // Should be able to claim both seeds
    let t1 = mgr.db.claim_next(1).unwrap().pop().unwrap();
    let t2 = mgr.db.claim_next(1).unwrap().pop().unwrap();
    dbg!(t1.url.clone(), t2.url.clone());
    assert!(t1.url == "http://foo.com/" || t1.url == "http://bar.com/");
    assert!(t2.url == "http://foo.com/" || t2.url == "http://bar.com/");
    assert_ne!(t1.url, t2.url);
}

#[tokio::test]
async fn test_process_discovered_links_batching_and_filtering() {
    setup_test_logging();
    setup_test_config();

    let mut mgr = setup_manager();
    let msg = DiscoveredLinks {
        links: vec![
            DiscoveredLink {
                url: "http://foo.com/page1".to_string(),
                priority: Priority::default(),
            },
            DiscoveredLink {
                url: "http://baz.com/page2".to_string(),
                priority: Priority::default(),
            }, // not allowed
            DiscoveredLink {
                url: "ftp://foo.com/file".to_string(),
                priority: Priority::default(),
            }, // not http
        ],
        depth: 1,
        parent_url_id: 1,
    };
    add_seeds(&mgr, &[]);
    mgr.process_discovered_links(msg).await;
    let _ = dump_table(&mgr, "urls");
    let _ = dump_table(&mgr, "frontier");
    // Only http://foo.com/page1 should be enqueued
    let t = mgr.db.claim_next(1).unwrap().pop().unwrap();
    assert_eq!(t.url, "http://foo.com/page1");
    assert_eq!(t.depth, 1);
    assert_eq!(t.discovered_from, Some(1));
    // No more tasks
    assert!(mgr.db.claim_next(1).unwrap().pop().is_none());
}
