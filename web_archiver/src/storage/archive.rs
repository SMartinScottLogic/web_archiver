use crate::frontier::db::frontier::FrontierDb;
use anyhow::Result;
use common::{Archiver, historical::HistoricalPage};
use tokio::sync::mpsc::Receiver;
use tracing::{error, info};

pub async fn storage_loop(
    archiver: impl Archiver,
    mut rx: Receiver<HistoricalPage>,
    db: FrontierDb,
) {
    while let Some(page) = rx.recv().await {
        match store_page(&archiver, &page) {
            Ok(_) => {
                // Mark as complete in the DB
                if let Err(e) = db.mark_complete(page.task.url_id) {
                    error!("Failed to mark complete for {}: {}", page.task.url, e);
                }
            }
            Err(e) => {
                error!("Failed to store {}: {}", page.task.url, e);
            }
        }
    }
}

fn store_page(archiver: &impl Archiver, page: &HistoricalPage) -> Result<()> {
    let outpath = archiver.store_page(page)?;
    info!("Stored page: {} -> {:?}", page.task.url, outpath);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::MockArchiver;
    use common::historical::{HistoricalContent, HistoricalSnapshot};
    use common::types::{FetchTask, PageMetadata};
    use std::collections::{HashSet, VecDeque};
    use std::path::PathBuf;

    #[test]
    fn test_store_page_creates_file() {
        let page = HistoricalPage {
            task: FetchTask {
                article_id: 0,
                url_id: 1,
                url: "http://foo.com/test".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            current: Some(HistoricalSnapshot {
                content_markdown: vec![HistoricalContent {
                    page: 1,
                    content: common::historical::HistoricalContentType::Literal(
                        "content".to_string(),
                    ),
                }],
                links: HashSet::new(),
                metadata: Some(PageMetadata {
                    status_code: 200,
                    content_type: Some("text/html".to_string()),
                    fetch_time: 0,
                    title: Some("Test".to_string()),
                    document_metadata: Some(vec![]),
                }),
            }),
            historical_snapshots: VecDeque::new(),
            all_links: HashSet::new(),
        };
        let mut archiver = MockArchiver::new();
        archiver
            .expect_store_page()
            //.with(predicate::eq(page.clone())) // TODO Check if this is required?
            .return_once(|_| Ok(PathBuf::from("fake/page/location/file.json")));
        let result = store_page(&archiver, &page);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_storage_loop_marks_complete() {
        use crate::frontier::db::frontier::FrontierDb;
        use rusqlite::Connection;
        use std::sync::{Arc, Mutex};
        use tokio::sync::mpsc;

        // Setup in-memory DB with minimal schema
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
        let db = FrontierDb {
            conn: Arc::new(Mutex::new(conn)),
        };

        // Insert a test URL and frontier row
        let url = "http://foo.com/test";
        let url_id = {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO urls (url, domain, discovered_at) VALUES (?1, ?2, 0)",
                rusqlite::params![url, "foo.com"],
            )
            .unwrap();
            conn.query_row(
                "SELECT id FROM urls WHERE url = ?1",
                rusqlite::params![url],
                |row| row.get(0),
            )
            .unwrap()
        };

        // Insert a test article
        let article_id = {
            0
        };
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO frontier (url_id, priority, depth, discovered_from, status) VALUES (?1, 0, 0, NULL, 'pending')",
                rusqlite::params![url_id],
            ).unwrap();
        }

        // Setup channel and send ExtractedPage
        let (tx, rx) = mpsc::channel(1);
        let page = HistoricalPage {
            task: FetchTask {
                article_id,
                url_id,
                url: url.to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            current: Some(HistoricalSnapshot {
                content_markdown: vec![HistoricalContent {
                    page: 1,
                    content: common::historical::HistoricalContentType::Literal(
                        "content".to_string(),
                    ),
                }],
                links: HashSet::new(),
                metadata: Some(PageMetadata {
                    status_code: 200,
                    content_type: Some("text/html".to_string()),
                    fetch_time: 0,
                    title: Some("Test".to_string()),
                    document_metadata: Some(vec![]),
                }),
            }),
            all_links: HashSet::new(),
            historical_snapshots: VecDeque::new(),
        };
        tx.send(page).await.unwrap();
        drop(tx); // Close channel

        let mut archiver = MockArchiver::new();
        archiver
            .expect_store_page()
            .returning(move |_page| Ok(PathBuf::from(format!("fake/page/{}", url_id))));

        storage_loop(archiver, rx, db.clone()).await;

        // Check that the status is now 'complete'
        let conn = db.conn.lock().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM frontier WHERE url_id = ?1",
                rusqlite::params![url_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "complete");
    }
}
