use anyhow::Context;
use common::types::{ArticleId, Priority};
use common::url::extract_domain;
use common::{types::FetchTask, url::remove_pagination_params};
use rusqlite::{Connection, OptionalExtension, Result, Transaction, params};
use std::sync::{Arc, Mutex};
use tracing::{debug, debug_span, error};
use url::Url;

use crate::settings::CONFIG;

#[cfg_attr(test, mockall::automock)]
pub trait FrontierDbTrait: Send + Sync + 'static {
    fn connect(conn: Arc<Mutex<Connection>>) -> Self;
    fn enqueue_batch(&self, batch: &[FetchTask], high_priority: bool) -> Result<(), anyhow::Error>;
    fn mark_url(&self, url_id: i64, status: &str) -> Result<(), anyhow::Error>;
    fn fail_url(&self, url_id: i64, retryable: bool) -> Result<bool, anyhow::Error>;
    fn url_is_skipped(&self, url: &str) -> Result<bool, anyhow::Error>;
    fn mark_article(&self, article_id: ArticleId, status: &str) -> Result<(), anyhow::Error>;
}
#[derive(Clone)]
pub struct FrontierDb {
    pub conn: Arc<Mutex<Connection>>,
}

fn enqueue_frontier_row(
    tx: &Transaction<'_>,
    url_id: i64,
    task: &FetchTask,
    force_priority: bool,
    excluded: bool,
) -> Result<()> {
    tx.execute(
        r#"INSERT INTO frontier (url_id, priority, depth, discovered_from, status)
        VALUES (?1, ?2, ?3, ?4, CASE WHEN ?6 THEN 'skipped' ELSE 'pending' END)
        ON CONFLICT(url_id) DO UPDATE SET
            depth = MIN(frontier.depth, excluded.depth),
            priority = CASE
                WHEN ?5 THEN excluded.priority
                WHEN excluded.priority > frontier.priority THEN excluded.priority
                ELSE frontier.priority
            END,
            status = CASE
                WHEN ?6 THEN 'skipped'
                WHEN excluded.priority = ?7
                    AND frontier.status IN ('complete', 'failed_terminal') THEN 'pending'
                ELSE frontier.status
            END,
            attempt_count = CASE
                WHEN excluded.priority = ?7
                    AND frontier.status IN ('complete', 'failed_terminal') THEN 0
                ELSE frontier.attempt_count
            END,
            next_attempt_at = CASE
                WHEN excluded.priority = ?7
                    AND frontier.status IN ('complete', 'failed_terminal') THEN NULL
                ELSE frontier.next_attempt_at
            END;
        "#,
        params![
            url_id,
            task.priority,
            task.depth,
            task.discovered_from,
            force_priority,
            excluded,
            Priority::Article
        ],
    )?;
    Ok(())
}

impl FrontierDb {
    /// Reset 'in_progress' tasks to 'pending'
    pub fn reset_in_progress(&self) -> Result<usize> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE frontier
             SET status = CASE
                    WHEN COALESCE(attempt_count, 0) >= 5 THEN 'failed_terminal'
                    ELSE 'pending'
                 END,
                 priority = ?1
             WHERE status = 'in_progress'",
            params![Priority::default()],
        )?;
        tx.commit()?;
        Ok(updated)
    }

    /// Reset ALL tasks to 'pending'
    pub fn reset_all(&self) -> Result<usize> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE frontier
             SET status = 'pending', priority = ?1, attempt_count = 0, next_attempt_at = NULL",
            params![Priority::default()],
        )?;
        tx.commit()?;
        Ok(updated)
    }

    /// Re-queue stale completed HTTP URLs and failed URLs whose retry delay elapsed.
    pub fn requeue_stale_or_failed(&self, refetch_after_days: u64) -> anyhow::Result<usize> {
        const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
        let age_seconds = refetch_after_days
            .checked_mul(SECONDS_PER_DAY)
            .and_then(|seconds| i64::try_from(seconds).ok())
            .ok_or_else(|| anyhow::anyhow!("refetch age exceeds the supported range"))?;
        let cutoff = chrono::Utc::now()
            .timestamp()
            .checked_sub(age_seconds)
            .ok_or_else(|| anyhow::anyhow!("refetch age exceeds the supported range"))?;

        let conn = self.conn.lock().unwrap();
        let retried = conn.execute(
            "UPDATE frontier
             SET status = 'pending',
                 next_attempt_at = NULL
             WHERE status = 'failed'
                AND COALESCE(attempt_count, 0) < 5
                AND COALESCE(next_attempt_at, 0) <= CAST(strftime('%s', 'now') AS INTEGER)",
            [],
        )?;
        let stale = conn.execute(
            "UPDATE frontier
             SET status = 'pending',
                 next_attempt_at = NULL
             WHERE status = 'complete'
                AND latest_fetch_time <= ?1
                AND url_id IN (
                    SELECT id FROM urls WHERE use_playwright = 0
                )",
            params![cutoff],
        )?;
        Ok(retried + stale)
    }

    /// Batch insert fetch tasks (deduplication by URL)
    pub fn enqueue_batch(&self, tasks: &[FetchTask], force_priority: bool) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for task in tasks {
            let parsed_url = Url::parse(&task.url).ok();
            let domain = parsed_url
                .as_ref()
                .and_then(Url::host_str)
                .map(str::to_owned)
                .or_else(|| extract_domain(&task.url))
                .unwrap_or_default();
            let matching_hosts = CONFIG.get().map(|config| {
                config
                    .hosts
                    .iter()
                    .filter(|host| host.domains.iter().any(|configured| configured == &domain))
                    .collect::<Vec<_>>()
            });
            let (use_playwright, excluded, route_configured) = match matching_hosts {
                Some(hosts) if !hosts.is_empty() => {
                    let path = parsed_url.as_ref().map(Url::path).unwrap_or_default();
                    (
                        hosts.iter().any(|host| host.use_playwright),
                        hosts.iter().any(|host| host.excludes_path(path)),
                        true,
                    )
                }
                _ => (task.use_playwright, false, false),
            };
            if excluded {
                debug!(url = %task.url, "Rejecting URL due to configured path exclusion");
            }

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
                "INSERT INTO urls (url, domain, discovered_at, article_id, use_playwright)
                 VALUES (?1, ?2, strftime('%s','now'), ?3, ?4)
                 ON CONFLICT(url) DO UPDATE SET use_playwright =
                    CASE WHEN ?5 THEN excluded.use_playwright ELSE urls.use_playwright END",
                params![
                    &task.url,
                    &domain,
                    article_id,
                    use_playwright,
                    route_configured
                ],
            )?;
            let url_id: i64 = tx.query_row(
                "SELECT id FROM urls WHERE url = ?1",
                params![&task.url],
                |row: &rusqlite::Row<'_>| row.get(0),
            )?;
            enqueue_frontier_row(&tx, url_id, task, force_priority, excluded)?;
        }
        tx.commit()?;
        Ok(())
    }

    #[allow(dead_code)]
    const GET_NEXT_BALANCED_SLOW: &'static str = r#"
    SELECT url_id, url, article_id, depth, priority, discovered_from, use_playwright, domain
         FROM (
             SELECT 
                 f.url_id,
                 u.url,
                 u.article_id,
                 f.depth,
                 f.priority,
                 f.discovered_from,
                 u.use_playwright,
                 u.domain,
                 ROW_NUMBER() OVER (
                     PARTITION BY u.domain
                     ORDER BY (f.priority - f.depth) DESC
                 ) as rn
             FROM frontier f
             JOIN urls u ON f.url_id = u.id
             WHERE f.status = 'pending'
               AND u.use_playwright = 0
               AND COALESCE(f.attempt_count, 0) < 5
               AND COALESCE(f.next_attempt_at, 0) <= CAST(strftime('%s', 'now') AS INTEGER)
         )
         WHERE rn = 1
         LIMIT ?1"#;
    #[allow(dead_code)]
    const GET_NEXT_FAST: &'static str = r#"
    SELECT f.url_id, u.url, u.article_id, f.depth, f.priority, f.discovered_from, u.use_playwright
    FROM frontier f JOIN urls u ON f.url_id = u.id 
    WHERE f.status = 'pending'
      AND u.use_playwright = 0
      AND COALESCE(f.attempt_count, 0) < 5
      AND COALESCE(f.next_attempt_at, 0) <= CAST(strftime('%s', 'now') AS INTEGER)
    ORDER BY (f.priority-f.depth) DESC LIMIT ?1"#;
    /// Atomically claim the next pending task for fetching
    pub fn claim_next(&self, limit: isize) -> Result<Vec<FetchTask>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let tasks = {
            let mut stmt = tx
                .prepare(Self::GET_NEXT_FAST)
                .inspect_err(|e| error!("Failed to get next url: {:?}", e))?;
            stmt.query_map([limit], |row| {
                Ok(FetchTask {
                    url_id: row.get(0)?,
                    url: row.get(1)?,
                    article_id: row.get(2)?,
                    depth: row.get(3)?,
                    priority: row.get(4)?,
                    discovered_from: row.get(5)?,
                    use_playwright: row.get(6)?,
                })
            })
            .inspect_err(|e| error!("Failed to get next url: {:?}", e))?
            .collect::<Result<Vec<_>>>()
            .inspect_err(|e| error!("Failed to get next url: {:?}", e))?
        };
        for task in &tasks {
            tx.execute(
                "UPDATE frontier
                 SET status = 'in_progress',
                     claimed_at = strftime('%s','now'),
                     attempt_count = COALESCE(attempt_count, 0) + 1
                 WHERE url_id = ?1",
                params![task.url_id],
            )?;
        }
        tx.commit()?;
        Ok(tasks)
    }

    /// Count the number of fetched pages (status = 'complete')
    pub fn count_fetched(&self) -> Result<u64> {
        let conn = {
            let lock_span = debug_span!("sqlite_count_fetched_connection_lock");
            let _lock_guard = lock_span.enter();
            self.conn.lock().unwrap()
        };
        let query_span = debug_span!("sqlite_count_fetched");
        let _query_guard = query_span.enter();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM frontier WHERE status = 'complete'",
            [],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Count the number of pending or in-progress pages
    pub fn count_pending(&self) -> Result<u64> {
        let conn = {
            let lock_span = debug_span!("sqlite_count_pending_connection_lock");
            let _lock_guard = lock_span.enter();
            self.conn.lock().unwrap()
        };
        let query_span = debug_span!("sqlite_count_pending");
        let _query_guard = query_span.enter();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM frontier WHERE status = 'pending' OR status = 'in_progress'",
            [],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Set a URL's frontier status.
    pub fn mark(&self, url_id: i64, status: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE frontier
             SET status = ?2,
                 attempt_count = CASE WHEN ?2 = 'complete' THEN 0 ELSE attempt_count END,
                 next_attempt_at = CASE WHEN ?2 = 'complete' THEN NULL ELSE next_attempt_at END,
                 latest_fetch_time = CASE
                     WHEN ?2 = 'complete' THEN strftime('%s', 'now')
                     ELSE latest_fetch_time
                 END
             WHERE url_id = ?1",
            params![url_id, status],
        )?;
        Ok(())
    }

    /// Record a fetch failure and indicate whether this URL has exhausted retries.
    pub fn fail(&self, url_id: i64, retryable: bool) -> Result<bool> {
        const MAX_ATTEMPTS: i64 = 5;

        let conn = self.conn.lock().unwrap();
        let attempt_count: Option<i64> = conn
            .query_row(
                "SELECT attempt_count FROM frontier WHERE url_id = ?1",
                params![url_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(attempt_count) = attempt_count else {
            return Ok(true);
        };

        let terminal = !retryable || attempt_count >= MAX_ATTEMPTS;
        if terminal {
            conn.execute(
                "UPDATE frontier SET status = 'failed_terminal', next_attempt_at = NULL WHERE url_id = ?1",
                params![url_id],
            )?;
        } else {
            let delay_minutes = 1_i64
                .checked_shl((attempt_count.saturating_sub(1)).min(3) as u32)
                .unwrap_or(8);
            let delay_seconds = delay_minutes * 60;
            conn.execute(
                "UPDATE frontier
                 SET status = 'failed',
                     next_attempt_at = strftime('%s', 'now') + ?2
                 WHERE url_id = ?1",
                params![url_id, delay_seconds],
            )?;
        }
        Ok(terminal)
    }

    pub fn url_is_skipped(&self, url: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT frontier.status = 'skipped'
             FROM frontier JOIN urls ON urls.id = frontier.url_id
             WHERE urls.url = ?1",
            params![url],
            |row| row.get(0),
        )
    }

    /// Set the frontier status for all URLs in an article.
    pub fn mark_article(&self, article_id: i64, status: &str) -> Result<()> {
        let conn = {
            let lock_span = debug_span!("sqlite_connection_lock", article_id);
            let _lock_guard = lock_span.enter();
            self.conn.lock().unwrap()
        };

        let query_span = debug_span!("sqlite_mark_article", article_id, status);
        let _query_guard = query_span.enter();
        conn.execute(
            r#"UPDATE frontier
            SET status = ?2,
                attempt_count = CASE WHEN ?2 = 'complete' THEN 0 ELSE attempt_count END,
                next_attempt_at = CASE WHEN ?2 = 'complete' THEN NULL ELSE next_attempt_at END,
                latest_fetch_time = CASE
                    WHEN ?2 = 'complete' THEN strftime('%s', 'now')
                    ELSE latest_fetch_time
                END
            WHERE url_id IN (
                SELECT id
                FROM urls
                WHERE article_id = ?1
            );"#,
            params![article_id, status],
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

    fn mark_url(&self, url_id: i64, status: &str) -> Result<(), anyhow::Error> {
        self.mark(url_id, status)
            .with_context(|| format!("mark url {status}"))
    }

    fn fail_url(&self, url_id: i64, retryable: bool) -> Result<bool, anyhow::Error> {
        self.fail(url_id, retryable)
            .with_context(|| format!("record url {url_id} failure"))
    }

    fn url_is_skipped(&self, url: &str) -> Result<bool, anyhow::Error> {
        self.url_is_skipped(url)
            .with_context(|| format!("check whether url is skipped: {url}"))
    }

    fn mark_article(&self, article_id: ArticleId, status: &str) -> Result<(), anyhow::Error> {
        self.mark_article(article_id, status)
            .with_context(|| format!("mark article {status}"))
    }
}
