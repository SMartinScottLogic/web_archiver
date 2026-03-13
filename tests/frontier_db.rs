//! Unit tests for the FrontierDb (database-backed queue)

use rusqlite::{Connection};
use std::sync::{Arc, Mutex};
use web_archiver::frontier::db::frontier::FrontierDb;
use web_archiver::types::messages::FetchTask;

fn setup_db() -> FrontierDb {
    let conn = Connection::open_in_memory().unwrap();
    // Create minimal schema for testing
    conn.execute_batch(r#"
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
    "#).unwrap();
    FrontierDb { conn: Arc::new(Mutex::new(conn)) }
}

#[test]
fn test_enqueue_and_claim() {
    let db = setup_db();
    let task = FetchTask {
        url_id: 0,
        url: "http://example.com".to_string(),
        depth: 0,
        priority: 5,
        discovered_from: None,
    };
    db.enqueue_batch(std::slice::from_ref(&task)).unwrap();
    let claimed = db.claim_next().unwrap().unwrap();
    assert_eq!(claimed.url, task.url);
    assert_eq!(claimed.depth, task.depth);
    assert_eq!(claimed.priority, task.priority);
    assert_eq!(claimed.discovered_from, task.discovered_from);
}

#[test]
fn test_enqueue_batch_deduplication() {
    let db = setup_db();
    let t1 = FetchTask {
        url_id: 0,
        url: "http://a.com".to_string(),
        depth: 0,
        priority: 1,
        discovered_from: None,
    };
    let t2 = FetchTask {
        url_id: 0,
        url: "http://b.com".to_string(),
        depth: 1,
        priority: 2,
        discovered_from: Some(1),
    };
    let t3 = FetchTask {
        url_id: 0,
        url: "http://a.com".to_string(), // duplicate
        depth: 2,
        priority: 3,
        discovered_from: Some(2),
    };
    db.enqueue_batch(&[t1.clone(), t2.clone(), t3.clone()]).unwrap();
    // Only two unique URLs should be present
    let mut seen = vec![];
    for _ in 0..2 {
        let t = db.claim_next().unwrap().unwrap();
        seen.push(t.url.clone());
        db.mark_complete(t.url_id).unwrap();
    }
    assert!(seen.contains(&t1.url));
    assert!(seen.contains(&t2.url));
    // No more tasks
    assert!(db.claim_next().unwrap().is_none());
}

#[test]
fn test_mark_complete_and_counts() {
    let db = setup_db();
    let t1 = FetchTask {
        url_id: 0,
        url: "http://foo.com".to_string(),
        depth: 0,
        priority: 1,
        discovered_from: None,
    };
    let t2 = FetchTask {
        url_id: 0,
        url: "http://bar.com".to_string(),
        depth: 0,
        priority: 2,
        discovered_from: None,
    };
    db.enqueue_batch(&[t1.clone(), t2.clone()]).unwrap();
    let c1 = db.claim_next().unwrap().unwrap();
    db.mark_complete(c1.url_id).unwrap();
    assert_eq!(db.count_fetched().unwrap(), 1);
    assert_eq!(db.count_pending().unwrap(), 1);
    let c2 = db.claim_next().unwrap().unwrap();
    db.mark_complete(c2.url_id).unwrap();
    assert_eq!(db.count_fetched().unwrap(), 2);
    assert_eq!(db.count_pending().unwrap(), 0);
}
