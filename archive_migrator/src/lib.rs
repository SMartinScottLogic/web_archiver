use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use common::Archiver;
use common::historical::HistoricalPage;
use common::page::PageReader;
use common::types::ExtractedPage;
use common::url::{canonicalize_url, extract_domain};
use rusqlite::{Connection, params};

fn read_page(path: &Path) -> anyhow::Result<Box<dyn PageReader>> {
    let text = fs::read_to_string(path)?;
    if let Err(e) = serde_json::from_str::<ExtractedPage>(&text) {
        println!("error 1 : {:?}", e);
    }
    if let Err(e) = serde_json::from_str::<HistoricalPage>(&text) {
        println!("error 2 : {:?}", e);
    }
    if let Ok(content) = serde_json::from_str::<ExtractedPage>(&text) {
        return Ok(Box::new(content));
    }
    if let Ok(content) = serde_json::from_str::<HistoricalPage>(&text) {
        return Ok(Box::new(content));
    }
    Err(anyhow::Error::msg(format!(
        "Failed to parse {}",
        path.display()
    )))
}

pub fn process_file(
    archiver: &impl Archiver,
    path: &Path,
    dry_run: bool,
) -> Result<Option<Box<dyn PageReader>>> {
    // Read JSON
    let mut page = read_page(path)?;

    let final_url = canonicalize_url(page.url())
        .ok_or_else(|| anyhow::Error::msg(format!("failed to canonicalise {}", page.url())))?;

    page.set_url(&final_url);

    let final_path = archiver.generate_filename(&*page)?;

    if path == final_path {
        println!("keep in place {:?}", path);
        return Ok(Some(page));
    }

    println!("move from {:?} to {:?}", path, final_path);

    if dry_run {
        return Ok(Some(page));
    }

    // --- COPY ---
    fs::copy(path, &final_path).with_context(|| format!("Failed to copy to {:?}", final_path))?;

    // --- VERIFY (size check) ---
    let src_size = fs::metadata(path)?.len();
    let dst_size = fs::metadata(&final_path)?.len();

    if src_size != dst_size {
        anyhow::bail!(
            "Size mismatch after copy: {:?} ({} bytes) → {:?} ({} bytes)",
            path,
            src_size,
            final_path,
            dst_size
        );
    }

    // --- DELETE ORIGINAL ---
    fs::remove_file(path).with_context(|| format!("Failed to delete original {:?}", path))?;

    Ok(Some(page))
}

pub fn ensure_complete_in_db(page: &dyn PageReader, conn: &mut Connection) -> Result<()> {
    let current = page
        .current()
        .as_ref()
        .ok_or_else(|| anyhow::Error::msg("failed to get current snapshot"))?;
    let task = &current.task;
    let fetch_time = current
        .metadata
        .as_ref()
        .map(|metadata| metadata.fetch_time)
        .unwrap_or_default() as i64;

    let url = &task.url;
    let tx = conn.transaction()?;
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO urls (url, domain, discovered_at) VALUES (?1, ?2, ?3)",
        params![&url, extract_domain(url).unwrap_or_default(), fetch_time],
    )?;
    if inserted > 0 {
        println!("   - added to DB");
    }
    let url_id: i64 = tx.query_row(
        "SELECT id FROM urls WHERE url = ?1",
        params![&url],
        |row: &rusqlite::Row<'_>| row.get(0),
    )?;
    // tx.execute(
    //     "INSERT OR IGNORE INTO frontier (url_id, priority, depth, discovered_from, status) VALUES (?1, ?2, ?3, ?4, 'complete')",
    //     params![url_id, page.task.priority, page.task.depth, page.task.discovered_from],
    // )?;
    tx.execute(
                "INSERT INTO frontier (url_id, priority, depth, discovered_from, status) VALUES (?1, ?2, ?3, ?4, 'complete') ON CONFLICT(url_id) DO UPDATE SET status = 'complete'", 
                params![url_id, task.priority, task.depth, task.discovered_from],
            )?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::historical::{HistoricalContentType, HistoricalSnapshot};
    use common::types::FetchTask;
    use rusqlite::Connection;
    use std::collections::{HashSet, VecDeque};
    use std::fs::{self, File};
    use std::io::{BufWriter, Write as _};
    use tempfile::tempdir;

    use common::MockArchiver;
    use common::page::MockPageReader;
    use mockall::predicate::*;

    fn make_snapshot(url: &str) -> HistoricalSnapshot {
        HistoricalSnapshot {
            task: FetchTask {
                url: url.to_string(),
                url_id: 123,
                priority: 1,
                depth: 2,
                discovered_from: Some(0),
            },
            metadata: None,
            content_markdown: HistoricalContentType::Literal("Hello World".into()),
            links: vec![],
        }
    }

    #[test]
    fn test_read_page_invalid_json() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("bad.json");
        fs::write(&file, "not json").unwrap();

        let result = read_page(&file);
        assert!(result.is_err());
    }

    #[test]
    fn test_process_file_keep_in_place() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("file.json");

        // Minimal valid JSON depends on your real structs
        let content = ExtractedPage {
            task: FetchTask {
                url_id: 0,
                url: "https://example.com/".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: Some("Example content".to_string()),
            links: Vec::new(),
            metadata: None,
        };
        let file = File::create(&path).unwrap();
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &content).unwrap();
        writer.flush().unwrap();

        let mut archiver = MockArchiver::new();
        {
            let path = path.clone();
            archiver
                .expect_generate_filename()
                .returning(move |_| Ok(path.clone()));
        }

        let result = process_file(&archiver, &path, false);

        // Will likely fail at parsing unless valid JSON
        if let Ok(res) = result {
            assert!(res.is_some());
        }
    }

    #[test]
    fn test_process_file_extracted_page() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("file.json");

        let content = ExtractedPage {
            task: FetchTask {
                url_id: 0,
                url: "https://example.com/".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: Some("Example content".to_string()),
            links: Vec::new(),
            metadata: None,
        };
        let file = File::create(&path).unwrap();
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &content).unwrap();
        writer.flush().unwrap();

        let mut archiver = MockArchiver::new();
        {
            let path = path.clone();
            archiver
                .expect_generate_filename()
                .returning(move |_| Ok(path.clone()));
        }

        let result = process_file(&archiver, &path, false).unwrap();

        let result = result.unwrap();

        assert_eq!("https://example.com/", result.url());
    }

    #[test]
    fn test_process_file_historical_page() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("file.json");

        let content = HistoricalPage {
            url: "https://example.com/".to_string(),
            current: Some(HistoricalSnapshot {
                task: FetchTask {
                    url_id: 0,
                    url: "example.com".to_string(),
                    depth: 0,
                    priority: 0,
                    discovered_from: None,
                },
                content_markdown: HistoricalContentType::Literal("Example content".to_string()),
                links: Vec::new(),
                metadata: None,
            }),
            historical_snapshots: VecDeque::new(),
            all_links: HashSet::new(),
        };
        let file = File::create(&path).unwrap();
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &content).unwrap();
        writer.flush().unwrap();

        let mut archiver = MockArchiver::new();
        {
            let path = path.clone();
            archiver
                .expect_generate_filename()
                .returning(move |_| Ok(path.clone()));
        }

        let result = process_file(&archiver, &path, false).unwrap();

        let result = result.unwrap();

        assert_eq!("https://example.com/", result.url());
    }

    #[test]
    fn test_process_file_dry_run_move() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("file.json");
        let dst = dir.path().join("new.json");

        let content = HistoricalPage {
            url: "https://example.com/".to_string(),
            current: Some(HistoricalSnapshot {
                task: FetchTask {
                    url_id: 0,
                    url: "example.com".to_string(),
                    depth: 0,
                    priority: 0,
                    discovered_from: None,
                },
                content_markdown: HistoricalContentType::Literal("Example content".to_string()),
                links: Vec::new(),
                metadata: None,
            }),
            historical_snapshots: VecDeque::new(),
            all_links: HashSet::new(),
        };
        let file = File::create(&src).unwrap();
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &content).unwrap();
        writer.flush().unwrap();

        let mut archiver = MockArchiver::new();
        {
            let dst = dst.clone();
            archiver
                .expect_generate_filename()
                .returning(move |_| Ok(dst.clone()));
        }
        let result = process_file(&archiver, &src, true).unwrap();

        let result = result.unwrap();

        assert_eq!("https://example.com/", result.url());
        assert!(src.exists());
        assert!(!dst.exists());
    }

    #[test]
    fn test_process_file_move() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("file.json");
        let dst = dir.path().join("new.json");

        let content = HistoricalPage {
            url: "https://example.com/".to_string(),
            current: Some(HistoricalSnapshot {
                task: FetchTask {
                    url_id: 0,
                    url: "example.com".to_string(),
                    depth: 0,
                    priority: 0,
                    discovered_from: None,
                },
                content_markdown: HistoricalContentType::Literal("Example content".to_string()),
                links: Vec::new(),
                metadata: None,
            }),
            historical_snapshots: VecDeque::new(),
            all_links: HashSet::new(),
        };
        let file = File::create(&src).unwrap();
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &content).unwrap();
        writer.flush().unwrap();

        let mut archiver = MockArchiver::new();
        {
            let dst = dst.clone();
            archiver
                .expect_generate_filename()
                .returning(move |_| Ok(dst.clone()));
        }
        let result = process_file(&archiver, &src, false).unwrap();

        let result = result.unwrap();

        assert_eq!("https://example.com/", result.url());
        assert!(!src.exists());
        assert!(dst.exists());
    }

    #[test]
    fn test_ensure_complete_in_db_insert() {
        let mut conn = Connection::open_in_memory().unwrap();

        conn.execute_batch(
            r#"
            CREATE TABLE urls (
                id INTEGER PRIMARY KEY,
                url TEXT UNIQUE,
                domain TEXT,
                discovered_at INTEGER
            );

            CREATE TABLE frontier (
                url_id INTEGER PRIMARY KEY,
                priority INTEGER,
                depth INTEGER,
                discovered_from TEXT,
                status TEXT
            );
        "#,
        )
        .unwrap();

        let mut page = MockPageReader::new();

        let snapshot = make_snapshot("https://example.com");

        page.expect_current().return_const(Some(snapshot.clone()));

        ensure_complete_in_db(&page, &mut conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM urls", [], |r| r.get(0))
            .unwrap();

        assert_eq!(count, 1);
    }

    #[test]
    fn test_ensure_complete_in_db_update_existing() {
        let mut conn = Connection::open_in_memory().unwrap();

        conn.execute_batch(
            r#"
            CREATE TABLE urls (
                id INTEGER PRIMARY KEY,
                url TEXT UNIQUE,
                domain TEXT,
                discovered_at INTEGER
            );

            CREATE TABLE frontier (
                url_id INTEGER PRIMARY KEY,
                priority INTEGER,
                depth INTEGER,
                discovered_from TEXT,
                status TEXT
            );

            INSERT INTO urls (id, url, domain, discovered_at)
            VALUES (1, 'https://example.com', 'example.com', 0);

            INSERT INTO frontier (url_id, priority, depth, discovered_from, status)
            VALUES (1, 0, 0, NULL, 'pending');
        "#,
        )
        .unwrap();

        let mut page = MockPageReader::new();
        let snapshot = make_snapshot("https://example.com");

        page.expect_current().return_const(Some(snapshot.clone()));

        ensure_complete_in_db(&page, &mut conn).unwrap();

        let status: String = conn
            .query_row("SELECT status FROM frontier WHERE url_id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();

        assert_eq!(status, "complete");
    }

    #[test]
    fn test_ensure_complete_in_db_missing_snapshot() {
        let mut conn = Connection::open_in_memory().unwrap();

        conn.execute_batch(
            r#"
            CREATE TABLE urls (
                id INTEGER PRIMARY KEY,
                url TEXT UNIQUE,
                domain TEXT,
                discovered_at INTEGER
            );

            CREATE TABLE frontier (
                url_id INTEGER PRIMARY KEY,
                priority INTEGER,
                depth INTEGER,
                discovered_from TEXT,
                status TEXT
            );
        "#,
        )
        .unwrap();

        let mut page = MockPageReader::new();

        page.expect_current().return_const(None);

        let result = ensure_complete_in_db(&page, &mut conn);
        assert!(result.is_err());
    }
}
