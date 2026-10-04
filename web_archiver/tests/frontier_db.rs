//! Unit tests for the FrontierDb (database-backed queue)

use common::types::{FetchTask, Priority};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use web_archiver::frontier::db::frontier::FrontierDb;

fn setup_db() -> FrontierDb {
    let conn = Connection::open_in_memory().unwrap();
    // Create minimal schema for testing
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
            latest_fetch_time INTEGER DEFAULT 0,
            FOREIGN KEY(url_id) REFERENCES urls(id),
            UNIQUE(url_id)
        );
    "#,
    )
    .unwrap();
    FrontierDb {
        conn: Arc::new(Mutex::new(conn)),
    }
}

#[test]
fn test_enqueue_and_claim() {
    let db = setup_db();
    let task = FetchTask {
        article_id: 0,
        url_id: 0,
        url: "http://example.com".to_string(),
        depth: 0,
        priority: Priority::default(),
        discovered_from: None,
        use_playwright: false,
    };
    db.enqueue_batch(std::slice::from_ref(&task), false)
        .unwrap();
    let claimed = db.claim_next(1).unwrap().pop().unwrap();
    assert_eq!(claimed.url, task.url);
    assert_eq!(claimed.depth, task.depth);
    assert_eq!(claimed.priority, task.priority);
    assert_eq!(claimed.discovered_from, task.discovered_from);
}

#[test]
fn test_enqueue_batch_deduplication() {
    let db = setup_db();
    let t1 = FetchTask {
        article_id: 0,
        url_id: 0,
        url: "http://a.com".to_string(),
        depth: 0,
        priority: Priority::default(),
        discovered_from: None,
        use_playwright: false,
    };
    let t2 = FetchTask {
        article_id: 0,
        url_id: 0,
        url: "http://b.com".to_string(),
        depth: 1,
        priority: Priority::default(),
        discovered_from: Some(1),
        use_playwright: false,
    };
    let t3 = FetchTask {
        article_id: 0,
        url_id: 0,
        url: "http://a.com".to_string(), // duplicate
        depth: 2,
        priority: Priority::default(),
        discovered_from: Some(2),
        use_playwright: false,
    };
    db.enqueue_batch(&[t1.clone(), t2.clone(), t3.clone()], false)
        .unwrap();
    // Only two unique URLs should be present
    let mut seen = vec![];
    for _ in 0..2 {
        let t = db.claim_next(1).unwrap().pop().unwrap();
        seen.push(t.url.clone());
        db.mark(t.url_id, "complete").unwrap();
    }
    assert!(seen.contains(&t1.url));
    assert!(seen.contains(&t2.url));
    // No more tasks
    assert!(db.claim_next(1).unwrap().pop().is_none());
}

#[test]
fn test_mark_complete_and_counts() {
    let db = setup_db();
    let t1 = FetchTask {
        article_id: 0,
        url_id: 0,
        url: "http://foo.com".to_string(),
        depth: 0,
        priority: Priority::default(),
        discovered_from: None,
        use_playwright: false,
    };
    let t2 = FetchTask {
        article_id: 0,
        url_id: 0,
        url: "http://bar.com".to_string(),
        depth: 0,
        priority: Priority::default(),
        discovered_from: None,
        use_playwright: false,
    };
    db.enqueue_batch(&[t1.clone(), t2.clone()], false).unwrap();
    let c1 = db.claim_next(1).unwrap().pop().unwrap();
    db.mark(c1.url_id, "complete").unwrap();
    assert_eq!(db.count_fetched().unwrap(), 1);
    assert_eq!(db.count_pending().unwrap(), 1);
    let c2 = db.claim_next(1).unwrap().pop().unwrap();
    db.mark(c2.url_id, "complete").unwrap();
    assert_eq!(db.count_fetched().unwrap(), 2);
    assert_eq!(db.count_pending().unwrap(), 0);
    let latest_fetch_time: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT latest_fetch_time FROM frontier WHERE url_id = ?1",
            [c2.url_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(latest_fetch_time > 0);
}

#[test]
fn test_mark_complete_article_records_latest_fetch_time() {
    let db = setup_db();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO articles (id, url) VALUES (?1, ?2)",
            (1_i64, "http://foo.com"),
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO urls (id, url, article_id) VALUES
                (1, 'http://foo.com/a', 1),
                (2, 'http://foo.com/b', 1);
             INSERT INTO frontier (url_id, priority, depth, status) VALUES
                (1, 0, 0, 'in_progress'),
                (2, 0, 0, 'in_progress');",
        )
        .unwrap();
    }

    db.mark_article(1, "complete").unwrap();

    let conn = db.conn.lock().unwrap();
    let completed_times = conn
        .prepare(
            "SELECT latest_fetch_time FROM frontier
             WHERE url_id IN (1, 2)
             ORDER BY url_id",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(completed_times.len(), 2);
    assert!(completed_times.iter().all(|fetch_time| *fetch_time > 0));
}

#[test]
fn test_non_complete_statuses_do_not_record_fetch_time() {
    let db = setup_db();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute_batch(
            "INSERT INTO urls (id, url, article_id) VALUES
                (1, 'http://foo.com/a', 1),
                (2, 'http://foo.com/b', 1);
             INSERT INTO frontier (url_id, priority, depth, status) VALUES
                (1, 0, 0, 'in_progress'),
                (2, 0, 0, 'in_progress');",
        )
        .unwrap();
    }

    db.mark(1, "failed").unwrap();
    db.mark(2, "skipped").unwrap();

    let conn = db.conn.lock().unwrap();
    let statuses_and_times = conn
        .prepare(
            "SELECT status, latest_fetch_time FROM frontier
             WHERE url_id IN (1, 2)
             ORDER BY url_id",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        statuses_and_times,
        vec![("failed".to_string(), 0), ("skipped".to_string(), 0),]
    );
}

#[test]
fn test_requeue_stale_or_failed_only_requeues_old_completed_http_urls() {
    const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

    let db = setup_db();
    let stale = FetchTask {
        article_id: 0,
        url_id: 0,
        url: "http://stale.example.com".to_string(),
        depth: 0,
        priority: Priority::default(),
        discovered_from: None,
        use_playwright: false,
    };
    let recent = FetchTask {
        url: "http://recent.example.com".to_string(),
        ..stale.clone()
    };
    db.enqueue_batch(&[stale, recent], false).unwrap();
    let claimed = db.claim_next(2).unwrap();
    for task in &claimed {
        db.mark(task.url_id, "complete").unwrap();
    }

    let old_fetch_time = chrono::Utc::now().timestamp() - 31 * SECONDS_PER_DAY;
    let recent_fetch_time = chrono::Utc::now().timestamp() - 29 * SECONDS_PER_DAY;
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE frontier SET latest_fetch_time = ?1 WHERE url_id = ?2",
            (
                old_fetch_time,
                claimed
                    .iter()
                    .find(|task| task.url.contains("stale"))
                    .unwrap()
                    .url_id,
            ),
        )
        .unwrap();
        conn.execute(
            "UPDATE frontier SET latest_fetch_time = ?1 WHERE url_id = ?2",
            (
                recent_fetch_time,
                claimed
                    .iter()
                    .find(|task| task.url.contains("recent"))
                    .unwrap()
                    .url_id,
            ),
        )
        .unwrap();
    }

    assert_eq!(db.requeue_stale_or_failed(30).unwrap(), 1);
    let stale_url_id = claimed
        .iter()
        .find(|task| task.url.contains("stale"))
        .unwrap()
        .url_id;
    let retained_fetch_time: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT latest_fetch_time FROM frontier WHERE url_id = ?1",
            [stale_url_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained_fetch_time, old_fetch_time);

    let requeued = db.claim_next(2).unwrap();
    assert_eq!(requeued.len(), 1);
    assert!(requeued[0].url.contains("stale"));

    let recent_status: String = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT status FROM frontier WHERE url_id = ?1",
            [claimed
                .iter()
                .find(|task| task.url.contains("recent"))
                .unwrap()
                .url_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(recent_status, "complete");
}

#[test]
fn test_requeue_stale_or_failed_rejects_unrepresentable_age() {
    let db = setup_db();
    assert!(db.requeue_stale_or_failed(u64::MAX).is_err());
}

#[test]
fn test_requeue_stale_or_failed_leaves_playwright_completed_urls_complete() {
    let db = setup_db();
    let task = FetchTask {
        article_id: 0,
        url_id: 0,
        url: "http://playwright.example.com".to_string(),
        depth: 0,
        priority: Priority::default(),
        discovered_from: None,
        use_playwright: true,
    };
    db.enqueue_batch(&[task], false).unwrap();

    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE frontier
             SET status = 'complete', latest_fetch_time = 1",
            [],
        )
        .unwrap();
    }

    assert_eq!(db.requeue_stale_or_failed(1).unwrap(), 0);

    let (status, latest_fetch_time): (String, i64) = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT status, latest_fetch_time FROM frontier",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "complete");
    assert_eq!(latest_fetch_time, 1);
}

#[test]
fn test_requeue_stale_or_failed_requeues_all_failed_urls() {
    let db = setup_db();
    let http_task = FetchTask {
        article_id: 0,
        url_id: 0,
        url: "http://failed-http.example.com".to_string(),
        depth: 0,
        priority: Priority::default(),
        discovered_from: None,
        use_playwright: false,
    };
    let playwright_task = FetchTask {
        url: "http://failed-playwright.example.com".to_string(),
        use_playwright: true,
        ..http_task.clone()
    };
    let skipped_task = FetchTask {
        url: "http://skipped.example.com".to_string(),
        ..http_task.clone()
    };
    db.enqueue_batch(&[http_task, playwright_task, skipped_task], false)
        .unwrap();

    let recent_fetch_time = chrono::Utc::now().timestamp();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute_batch(
            "UPDATE frontier SET status = 'failed'
             WHERE url_id IN (
                 SELECT id FROM urls WHERE url LIKE '%failed-%'
             );
             UPDATE frontier SET status = 'skipped'
             WHERE url_id IN (
                 SELECT id FROM urls WHERE url LIKE '%skipped.example.com'
             );",
        )
        .unwrap();
        conn.execute(
            "UPDATE frontier SET latest_fetch_time = ?1 WHERE status = 'failed'",
            [recent_fetch_time],
        )
        .unwrap();
    }

    assert_eq!(db.requeue_stale_or_failed(30).unwrap(), 2);

    let statuses = {
        let conn = db.conn.lock().unwrap();
        conn.prepare(
            "SELECT urls.url, frontier.status FROM frontier
             JOIN urls ON urls.id = frontier.url_id
             ORDER BY urls.url",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    };
    assert_eq!(
        statuses,
        vec![
            (
                "http://failed-http.example.com".to_string(),
                "pending".to_string()
            ),
            (
                "http://failed-playwright.example.com".to_string(),
                "pending".to_string()
            ),
            (
                "http://skipped.example.com".to_string(),
                "skipped".to_string()
            ),
        ]
    );

    let claimed_http = db.claim_next(1).unwrap().pop().unwrap();
    assert!(!claimed_http.use_playwright);
    assert_eq!(claimed_http.url, "http://failed-http.example.com");
    let http_status: String = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT status FROM frontier WHERE url_id = ?1",
            [claimed_http.url_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(http_status, "in_progress");
}
