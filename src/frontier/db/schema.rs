use rusqlite::{Connection, Result};

pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS urls (
            id INTEGER PRIMARY KEY,
            url TEXT NOT NULL UNIQUE,
            domain TEXT NOT NULL,
            discovered_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS frontier (
            url_id INTEGER PRIMARY KEY,
            priority INTEGER NOT NULL,
            depth INTEGER NOT NULL,
            discovered_from INTEGER,
            status TEXT NOT NULL DEFAULT 'pending',
            claimed_at INTEGER,
            attempt_count INTEGER DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS fetch_history (
            id INTEGER PRIMARY KEY,
            url_id INTEGER NOT NULL,
            fetch_time INTEGER NOT NULL,
            status_code INTEGER,
            success INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS archive (
            url_id INTEGER PRIMARY KEY,
            json_path TEXT NOT NULL,
            fetch_time INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS discovered_links (
            id INTEGER PRIMARY KEY,
            source_url_id INTEGER NOT NULL,
            target_url_id INTEGER NOT NULL,
            discovered_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_frontier_status_priority
        ON frontier(status, priority DESC);

        CREATE INDEX IF NOT EXISTS idx_urls_domain
        ON urls(domain);
        "#,
    )?;

    Ok(())
}
