use crate::types::messages::FetchTask;
use rusqlite::{Connection, Result, params};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct FrontierDb {
    pub conn: Arc<Mutex<Connection>>,
}

impl FrontierDb {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Insert a new fetch task if not already present (deduplication by URL)
    pub fn enqueue(&self, task: &FetchTask) -> Result<()> {
        // Insert URL if not present
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO urls (url, domain, discovered_at) VALUES (?1, ?2, strftime('%s','now'))",
            params![&task.url, crate::util::extract_domain(&task.url).unwrap_or_default()],
        )?;
        // Get url_id
        let url_id: i64 = conn.query_row(
            "SELECT id FROM urls WHERE url = ?1",
            params![&task.url],
            |row| row.get(0),
        )?;
        // Insert into frontier if not present
        conn.execute(
            "INSERT OR IGNORE INTO frontier (url_id, priority, depth, discovered_from, status) VALUES (?1, ?2, ?3, ?4, 'pending')",
            params![url_id, task.priority, task.depth, task.discovered_from],
        )?;
        Ok(())
    }

    /// Atomically claim the next pending task for fetching
    pub fn claim_next(&self) -> Result<Option<FetchTask>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let task_opt = {
            let mut stmt = tx.prepare(
                "SELECT f.url_id, u.url, f.depth, f.priority, f.discovered_from \
                 FROM frontier f JOIN urls u ON f.url_id = u.id \
                 WHERE f.status = 'pending' \
                 ORDER BY f.priority DESC LIMIT 1",
            )?;
            stmt.query_map([], |row| {
                Ok(FetchTask {
                    url_id: row.get(0)?,
                    url: row.get(1)?,
                    depth: row.get(2)?,
                    priority: row.get(3)?,
                    discovered_from: row.get(4)?,
                })
            })?
            .next()
            .transpose()?
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
}
