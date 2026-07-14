use anyhow::Result;
use common::{settings::Host, types::FetchTask};
use flate2::read::GzDecoder;
use rusqlite::{Connection, params};
use serde_json::Value;
use std::{
    collections::HashSet,
    io::{BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tracing::{debug, error, warn};
use url::Url;

use crate::{frontier::db::frontier::FrontierDbTrait, settings::CONFIG};

pub struct JsonPoller<DB>
where
    DB: FrontierDbTrait,
{
    frontier_db: Arc<DB>,
    json_db: Arc<JsonDb>,
}
impl<DB> JsonPoller<DB>
where
    DB: FrontierDbTrait,
{
    pub fn new(frontier_db: Arc<DB>, json_db: Arc<JsonDb>) -> Self {
        Self {
            frontier_db,
            json_db,
        }
    }
}
impl<DB> JsonPoller<DB>
where
    DB: FrontierDbTrait,
{
    pub fn poll_all(&self) {
        while let Ok((json_file, depth)) = self.json_db.claim_pending() {
            debug!(?json_file, "claimed");
            let _ = self
                .process(&json_file, depth)
                .inspect_err(|e| error!(?e, ?json_file, "failed to process"));
        }
    }

    fn process(&self, path: &Path, depth: u32) -> Result<()> {
        let reader = open_json_reader(path)?;
        let content: Value = serde_json::from_reader(reader)?;

        let mut links = HashSet::new();
        Self::get_links(&mut links, &content);

        debug!("process {:?} -> {:#?}", path, links);
        let batch = links
            .iter()
            .filter_map(|url| {
                if let Some(domain) = url.domain() {
                    let domains = get_matching_domains(&CONFIG.get().unwrap().hosts, domain);
                    let exclude = CONFIG
                        .get()
                        .cloned()
                        .map(|c| c.json.exclude)
                        .unwrap_or_default();
                    if domains
                        .iter()
                        .map(|d| d.name.clone())
                        .any(|n| exclude.contains(&n))
                    {
                        debug!(url = url.to_string(), ?domains, domain, "excluded");
                        None
                    } else {
                        let use_playwright = domains.iter().any(|host| host.use_playwright);
                        debug!(url = url.to_string(), ?domains, domain, "retained");
                        Some(FetchTask {
                            article_id: 0,
                            url_id: 0,
                            url: url.to_string(),
                            depth: depth + 1,
                            priority: Default::default(),
                            discovered_from: None,
                            use_playwright,
                        })
                    }
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        self.frontier_db.enqueue_batch(&batch, false)?;
        self.json_db.set_json_status(path, "processed")?;
        Ok(())
    }

    fn get_links(links: &mut HashSet<Url>, value: &Value) {
        match value {
            Value::Null => {}
            Value::Bool(_) => {}
            Value::Number(_) => {}
            Value::String(s) => {
                if let Some(url) = Self::to_link(s) {
                    links.insert(url);
                }
            }
            Value::Array(values) => {
                for value in values {
                    Self::get_links(links, value);
                }
            }
            Value::Object(map) => {
                for value in map.values() {
                    Self::get_links(links, value);
                }
            }
        };
    }

    fn to_link(s: &str) -> Option<Url> {
        if (s.starts_with("http:") || s.starts_with("https:"))
            && !s.contains(|c: char| c.is_whitespace() || c == '"' || c == '\'')
        {
            Url::parse(s).ok()
        } else {
            None
        }
    }
}

fn get_matching_domains<'a>(hosts: &'a [Host], domain: &str) -> Vec<&'a Host> {
    hosts
        .iter()
        .filter(|&host| host.domains.iter().any(|d| d == domain))
        .collect()
}

pub struct JsonDb {
    pub conn: Arc<Mutex<Connection>>,
}
impl JsonDb {
    pub fn connect(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn reset(&self) -> Result<usize> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let updated = tx
            .execute(
                "UPDATE json_queue SET status = 'pending' WHERE status = 'in_progress'",
                params![],
            )
            .inspect_err(|e| error!(?e, "reset failed"))?;
        tx.commit()?;
        Ok(updated)
    }

    // TODO Modify to use row id, rather than path
    pub fn set_json_status(&self, path: &Path, status: &str) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE json_queue SET status = ?2 WHERE path = ?1",
            params![path.to_string_lossy(), status],
        )
        .inspect_err(|e| error!(?e, "set json status failed"))?;
        tx.commit()?;

        Ok(())
    }

    pub fn claim_pending(&self) -> Result<(PathBuf, u32)> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let (filename, depth): (String, u32) = tx
            .query_row(
                r#"SELECT path, depth FROM json_queue WHERE status='pending' LIMIT 1"#,
                [],
                |row: &rusqlite::Row<'_>| Ok((row.get(0)?, row.get(1)?)),
            )
            .inspect_err(|e| {
                if *e == rusqlite::Error::QueryReturnedNoRows {
                    warn!(?e, "next json failed")
                } else {
                    error!(?e, "next json failed")
                }
            })?;
        tx.execute(
            "UPDATE json_queue SET status = 'in_progress' WHERE path = ?1",
            params![filename],
        )
        .inspect_err(|e| error!(?e, "set json to 'in progress' failed"))?;
        tx.commit()?;

        // TODO Also return row id
        Ok((PathBuf::from(filename), depth))
    }
}

fn open_json_reader(path: &Path) -> Result<Box<dyn Read>> {
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut magic = [0u8; 2];
    let bytes_read = reader.read(&mut magic)?;
    reader.seek(SeekFrom::Start(0))?;

    if bytes_read == 2 && magic == [0x1f, 0x8b] {
        Ok(Box::new(GzDecoder::new(reader)))
    } else {
        Ok(Box::new(reader))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use common::{
        settings::Host,
        types::{ArticleId, FetchTask},
    };
    use rusqlite::Connection;
    use serde_json::json;
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };
    use tempfile::NamedTempFile;
    use url::Url;

    /// Minimal mock for FrontierDbTrait
    struct MockFrontierDb {
        pub batches: Arc<Mutex<Vec<Vec<FetchTask>>>>,
    }

    impl MockFrontierDb {
        fn new() -> Self {
            Self {
                batches: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl FrontierDbTrait for MockFrontierDb {
        fn enqueue_batch(&self, batch: &[FetchTask], _dedupe: bool) -> Result<()> {
            self.batches.lock().unwrap().push(batch.to_vec());
            Ok(())
        }

        fn connect(_conn: Arc<Mutex<Connection>>) -> Self {
            todo!()
        }

        fn mark_complete_article(&self, _article_id: ArticleId) -> Result<(), anyhow::Error> {
            todo!()
        }
    }

    fn setup_json_db() -> JsonDb {
        let conn = Connection::open_in_memory().unwrap();

        conn.execute(
            r#"
            CREATE TABLE json_queue (
                path TEXT PRIMARY KEY,
                depth INTEGER NOT NULL,
                status TEXT NOT NULL
            )
            "#,
            [],
        )
        .unwrap();

        JsonDb::connect(Arc::new(Mutex::new(conn)))
    }

    #[test]
    fn to_link_accepts_valid_http_url() {
        let result = JsonPoller::<MockFrontierDb>::to_link("https://example.com/path");

        assert!(result.is_some());
        assert_eq!(result.unwrap().as_str(), "https://example.com/path");
    }

    #[test]
    fn to_link_rejects_non_http_url() {
        let result = JsonPoller::<MockFrontierDb>::to_link("ftp://example.com");

        assert!(result.is_none());
    }

    #[test]
    fn to_link_rejects_whitespace() {
        let result = JsonPoller::<MockFrontierDb>::to_link("https://example.com/some path");

        assert!(result.is_none());
    }

    #[test]
    fn get_links_recursively_extracts_unique_links() {
        let value = json!({
            "a": "https://example.com",
            "nested": {
                "b": "https://example.com", // duplicate
                "c": [
                    "https://rust-lang.org",
                    "not-a-url",
                    {
                        "d": "https://docs.rs"
                    }
                ]
            }
        });

        let mut links = std::collections::HashSet::new();
        JsonPoller::<MockFrontierDb>::get_links(&mut links, &value);

        let expected: std::collections::HashSet<Url> = vec![
            Url::parse("https://example.com").unwrap(),
            Url::parse("https://rust-lang.org").unwrap(),
            Url::parse("https://docs.rs").unwrap(),
        ]
        .into_iter()
        .collect();

        assert_eq!(links, expected);
    }

    #[test]
    fn get_matching_domains_returns_only_matches() {
        let hosts = vec![
            Host {
                domains: vec!["example.com".into()],
                use_playwright: false,
                ignore_robots: true,
                name: "Example".into(),
                pages: common::settings::PageType::None,
                max_depth: None,
            },
            Host {
                domains: vec!["rust-lang.org".into()],
                use_playwright: true,
                ignore_robots: true,
                name: "RustLang".into(),
                pages: common::settings::PageType::None,
                max_depth: None,
            },
        ];

        let matches = get_matching_domains(&hosts, "rust-lang.org");

        assert_eq!(matches.len(), 1);
        assert!(matches[0].use_playwright);
    }

    #[test]
    fn claim_pending_marks_row_in_progress() {
        let db = setup_json_db();

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO json_queue (path, depth, status) VALUES (?1, ?2, ?3)",
                params!["/tmp/test.json", 2u32, "pending"],
            )
            .unwrap();
        }

        let (path, depth) = db.claim_pending().unwrap();

        assert_eq!(path, PathBuf::from("/tmp/test.json"));
        assert_eq!(depth, 2);

        let status: String = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT status FROM json_queue WHERE path = ?1",
                params!["/tmp/test.json"],
                |row| row.get(0),
            )
            .unwrap()
        };

        assert_eq!(status, "in_progress");
    }

    #[test]
    fn set_json_status_updates_status() {
        let db = setup_json_db();

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO json_queue (path, depth, status) VALUES (?1, ?2, ?3)",
                params!["/tmp/test.json", 1u32, "pending"],
            )
            .unwrap();
        }

        db.set_json_status(Path::new("/tmp/test.json"), "processed")
            .unwrap();

        let status: String = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT status FROM json_queue WHERE path = ?1",
                params!["/tmp/test.json"],
                |row| row.get(0),
            )
            .unwrap()
        };

        assert_eq!(status, "processed");
    }

    #[test]
    fn reset_changes_in_progress_back_to_pending() {
        let db = setup_json_db();

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO json_queue (path, depth, status) VALUES (?1, ?2, ?3)",
                params!["/tmp/test.json", 1u32, "in_progress"],
            )
            .unwrap();
        }

        let updated = db.reset().unwrap();

        assert_eq!(updated, 1);

        let status: String = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT status FROM json_queue WHERE path = ?1",
                params!["/tmp/test.json"],
                |row| row.get(0),
            )
            .unwrap()
        };

        assert_eq!(status, "pending");
    }

    #[test]
    fn process_extracts_links_and_marks_processed() {
        // NOTE:
        // This test assumes CONFIG is initialized in your test environment.
        // If CONFIG is not globally settable in tests, you may want to
        // refactor process() to inject config dependency instead.

        let frontier = Arc::new(MockFrontierDb::new());
        let json_db = Arc::new(setup_json_db());

        let file = NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            r#"{
                "url": "https://example.com/article",
                "nested": ["https://rust-lang.org"]
            }"#,
        )
        .unwrap();

        {
            let conn = json_db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO json_queue (path, depth, status) VALUES (?1, ?2, ?3)",
                params![file.path().to_string_lossy(), 3u32, "pending"],
            )
            .unwrap();
        }

        let poller = JsonPoller::new(frontier.clone(), json_db.clone());

        let result = poller.process(file.path(), 3);

        assert!(result.is_ok());

        let batches = frontier.batches.lock().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 2);

        for task in &batches[0] {
            assert_eq!(task.depth, 4);
        }

        let status: String = {
            let conn = json_db.conn.lock().unwrap();
            conn.query_row(
                "SELECT status FROM json_queue WHERE path = ?1",
                params![file.path().to_string_lossy()],
                |row| row.get(0),
            )
            .unwrap()
        };

        assert_eq!(status, "processed");
    }
}
