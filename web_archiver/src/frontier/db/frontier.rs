use anyhow::Context;
use common::types::{ArticleId, Priority};
use common::url::extract_domain;
use common::{types::FetchTask, url::remove_pagination_params};
use rusqlite::{Connection, Result, params};
use std::sync::{Arc, Mutex};
use tracing::error;

#[cfg_attr(test, mockall::automock)]
pub trait FrontierDbTrait: Send + Sync + 'static {
    fn connect(conn: Arc<Mutex<Connection>>) -> Self;
    fn enqueue_batch(&self, batch: &[FetchTask], high_priority: bool) -> Result<(), anyhow::Error>;

    fn mark_complete_article(&self, article_id: ArticleId) -> Result<(), anyhow::Error>;
}
#[derive(Clone)]
pub struct FrontierDb {
    pub conn: Arc<Mutex<Connection>>,
}

impl FrontierDb {
    /// Reset 'in_progress' tasks to 'pending'
    pub fn reset_in_progress(&self) -> Result<usize> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE frontier SET status = 'pending', priority = ?1 WHERE status = 'in_progress'",
            params![Priority::default()],
        )?;
        tx.commit()?;
        Ok(updated)
    }

    /// Batch insert fetch tasks (deduplication by URL)
    pub fn enqueue_batch(&self, tasks: &[FetchTask], force_priority: bool) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for task in tasks {
            // Construct article url from page url
            let article_url = remove_pagination_params(&task.url);
            tx.execute(
                "INSERT OR IGNORE INTO articles (url) VALUES (?1)",
                params![&article_url],
            )?;
            let article_id: i64 = tx.query_row(
                "SELECT id FROM articles WHERE url = ?1",
                params![&article_url],
                |row: &rusqlite::Row<'_>| row.get(0),
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO urls (url, domain, discovered_at, article_id) VALUES (?1, ?2, strftime('%s','now'), ?3)",
                params![&task.url, extract_domain(&task.url).unwrap_or_default(), article_id],
            )?;
            let url_id: i64 = tx.query_row(
                "SELECT id FROM urls WHERE url = ?1",
                params![&task.url],
                |row: &rusqlite::Row<'_>| row.get(0),
            )?;
            tx.execute(
                r#"INSERT INTO frontier (url_id, priority, depth, discovered_from, status)
                VALUES (?1, ?2, ?3, ?4, 'pending')
                ON CONFLICT(url_id) DO UPDATE SET
                    depth = MIN(frontier.depth, excluded.depth),
                    priority = CASE
                        WHEN ?5 THEN excluded.priority
                        WHEN excluded.priority > frontier.priority THEN excluded.priority
                        ELSE frontier.priority
                    END;
                "#,
                params![
                    url_id,
                    task.priority,
                    task.depth,
                    task.discovered_from,
                    force_priority
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Atomically claim the next pending task for fetching
    pub fn claim_next(&self) -> Result<Option<FetchTask>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let task_opt = {
            let mut stmt = tx
                .prepare(
                    "SELECT f.url_id, u.url, u.article_id, f.depth, f.priority, f.discovered_from \
                 FROM frontier f JOIN urls u ON f.url_id = u.id \
                 WHERE f.status = 'pending' \
                 ORDER BY (f.priority-f.depth) DESC LIMIT 1",
                )
                .inspect_err(|e| error!("Failed to get next url: {:?}", e))?;
            stmt.query_map([], |row| {
                Ok(FetchTask {
                    url_id: row.get(0)?,
                    url: row.get(1)?,
                    article_id: row.get(2)?,
                    depth: row.get(3)?,
                    priority: row.get(4)?,
                    discovered_from: row.get(5)?,
                })
            })
            .inspect_err(|e| error!("Failed to get next url: {:?}", e))?
            .next()
            .transpose()
            .inspect_err(|e| error!("Failed to get next url: {:?}", e))?
        };
        if let Some(ref task) = task_opt {
            tx.execute(
                "UPDATE frontier SET status = 'in_progress', claimed_at = strftime('%s','now') WHERE url_id = ?1",
                params![task.url_id],
            )?;
        }
        tx.commit()?;
        Ok(task_opt)
    }

    /// Count the number of fetched pages (status = 'complete')
    pub fn count_fetched(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM frontier WHERE status = 'complete'",
            [],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Count the number of pending or in-progress pages
    pub fn count_pending(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM frontier WHERE status = 'pending' OR status = 'in_progress'",
            [],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Mark a URL as complete in the frontier
    pub fn mark_complete(&self, url_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE frontier SET status = 'complete' WHERE url_id = ?1",
            params![url_id],
        )?;
        Ok(())
    }

    /// Mark all URLs in an article as complete in the frontier
    pub fn mark_complete_article(&self, article_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"UPDATE frontier
            SET status = 'complete'
            WHERE url_id IN (
                SELECT id
                FROM urls
                WHERE article_id = ?1
            );"#,
            params![article_id],
        )?;
        Ok(())
    }
}

impl FrontierDbTrait for FrontierDb {
    fn connect(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    fn enqueue_batch(&self, batch: &[FetchTask], high_priority: bool) -> anyhow::Result<()> {
        self.enqueue_batch(batch, high_priority)
            .context("enqueuing")
    }

    fn mark_complete_article(&self, article_id: ArticleId) -> Result<(), anyhow::Error> {
        self.mark_complete_article(article_id)
            .context("mark complete")
    }
}
