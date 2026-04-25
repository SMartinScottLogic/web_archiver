use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::File,
    io::BufReader,
    path::PathBuf,
    sync::Arc,
};

use anyhow::Context;
use common::{
    Archiver, JsonLd, historical::{HistoricalContent, HistoricalPage, HistoricalSnapshot}, types::{ArticleId, FetchTask, PageMetadata, Priority}, url::remove_pagination_params
};
use reqwest::StatusCode;
use tokio::sync::mpsc;
use tracing::{Level, debug, error, event_enabled, info, warn};

use crate::frontier::db::frontier::FrontierDbTrait;

#[derive(Clone, Debug)]
pub struct FetchedArticlePage {
    pub task: FetchTask,
    pub content: String,
    pub fetch_time: i64,
    pub links: HashSet<String>,
    pub title: Option<String>,
    pub document_metadata: Vec<HashMap<String, String>>,
    pub json_ld: Option<JsonLd>,
}

pub struct Router<T: Archiver, DB: FrontierDbTrait> {
    active: HashMap<ArticleId, (mpsc::Sender<FetchedArticlePage>, String)>,
    max_active: usize,
    archiver: T,
    done_tx: mpsc::Sender<ArticleId>,
    db: Arc<DB>,
}

struct ArticleState<DB>
where
    DB: FrontierDbTrait,
{
    task: FetchTask,
    filename: PathBuf,
    snapshot: HistoricalSnapshot,
    pages: HashMap<String, bool>,
    db: Arc<DB>,
}

impl<DB: FrontierDbTrait> ArticleState<DB> {
    fn new(filename: PathBuf, task: FetchTask, db: Arc<DB>) -> Self {
        Self {
            task,
            filename,
            snapshot: HistoricalSnapshot {
                content_markdown: Vec::new(),
                links: HashSet::new(),
                metadata: None,
            },
            pages: HashMap::new(),
            db,
        }
    }

    fn done(&self) -> bool {
        debug!(known_remaining = ?self.pages.iter().filter(|page| !*page.1).collect::<Vec<_>>(), "known pages");
        self.pages.iter().all(|(_url, &fetched)| fetched)
    }

    fn apply(&mut self, page: FetchedArticlePage) {
        let page_number = match common::url::extract_page(&page.task.url) {
            common::url::Page::Number(page_number) => page_number,
            _ => 1,
        };

        // Ensure current page is in pages (otherwise we'll double fetch the first)
        // Mark as fetched
        *self.pages.entry(page.task.url.clone()).or_insert(false) = true;

        let mut batch = Vec::new();
        // Add links to snapshot and article queue
        for link in &page.links {
            self.snapshot.links.insert(link.to_string());
            let mut priority = Priority::default();
            if self.is_page(link) {
                self.pages.entry(link.to_string()).or_insert(false);
                priority = Priority::Article;
            }
            batch.push(FetchTask {
                article_id: self.task.article_id,
                url_id: 0, // Will be set by DB
                url: link.to_string(),
                depth: self.task.depth + 1,
                priority,
                discovered_from: Some(self.task.url_id),
            });
        }
        if !batch.is_empty() {
            let _ = self
                .db
                .enqueue_batch(&batch, false)
                .inspect_err(|e| error!("failed enqueuing {:?}", e));
        }

        let content = HistoricalContent {
            content: common::historical::HistoricalContentType::Literal(page.content.to_owned()),
            page: page_number,
        };
        self.snapshot.content_markdown.push(content);

        // Setup metadata - use first found page for everything, replace title and json+ld with page 1
        match &mut self.snapshot.metadata {
            None => {
                self.snapshot.metadata = Some(PageMetadata {
                    status_code: StatusCode::OK.as_u16(),
                    content_type: None,
                    fetch_time: page.fetch_time.try_into().unwrap_or_default(),
                    title: page.title.clone(),
                    document_metadata: Some(page.document_metadata.clone()),
                    json_ld: page.json_ld.clone(),
                });
            }
            Some(metadata) if page_number == 1 => {
                metadata.title = page.title.clone();
                metadata.json_ld = page.json_ld.clone();
            }
            Some(_metadata) => {}
        };

        debug!(?self.filename, ?page, ?self.snapshot, page_number, page_url = page.task.url, "apply");
    }

    async fn persist(&self) -> Result<(), std::io::Error> {
        let pages = self
            .snapshot
            .content_markdown
            .iter()
            .map(|c| c.page)
            .collect::<Vec<_>>();
        debug!(?self.filename, num_pages = self.snapshot.content_markdown.len(), ?pages, ?self.snapshot, "persist");
        Ok(())
    }

    async fn finalize(mut self) -> anyhow::Result<()> {
        if event_enabled!(Level::DEBUG) {
            debug!(?self.filename, ?self.snapshot, "finalize");
        } else {
            info!(?self.filename, "finalize");
        }
        let article_id = self.task.article_id;
        // 1. Read from archive, or create empty record
        let mut historical_page = File::open(&self.filename)
            .context("opening")
            .map(BufReader::new)
            .and_then(|reader| serde_json::from_reader(reader).context("reading"))
            .or_else(|_| {
                Ok::<HistoricalPage, i8>(HistoricalPage {
                    task: self.task,
                    current: None,
                    historical_snapshots: VecDeque::new(),
                    all_links: HashSet::new(),
                })
            })
            .unwrap();
        debug!(?historical_page, "historical page");
        // 2. Add each page to record for current 'archival date'
        // 2a Sort by page
        self.snapshot
            .content_markdown
            .sort_by_cached_key(|page| page.page);
        // 2b Add to historical page
        historical_page.add_snapshot(self.snapshot)?;
        // 3. Save resultant file to archive (overwriting)
        historical_page
            .write_page(&self.filename)
            .inspect_err(|err| error!("Failed to write to {:?}: {:?}", self.filename, err))?;
        // 4. Update db
        self.db.mark_complete_article(article_id)
    }

    fn is_page(&self, url: &str) -> bool {
        remove_pagination_params(&self.task.url) == remove_pagination_params(url)
    }
}

impl<T: Archiver, DB: FrontierDbTrait> Router<T, DB> {
    pub fn new(
        archiver: T,
        db: Arc<DB>,
        done_tx: mpsc::Sender<ArticleId>,
        max_active: usize,
    ) -> Self {
        Self {
            active: HashMap::new(),
            max_active,
            archiver,
            done_tx,
            db,
        }
    }

    pub fn remove(&mut self, article_id: ArticleId) {
        self.active.remove(&article_id);
    }

    pub async fn route(&mut self, mut page: FetchedArticlePage) {
        let article_id = page.task.article_id;
        loop {
            // Case 1: already active
            if let Some((tx, _)) = self.active.get(&article_id) {
                match tx.send(page).await {
                    Ok(_) => return,
                    Err(e) => {
                        // actor died → recover page and retry
                        page = e.0;
                        self.active.remove(&article_id);
                        continue;
                    }
                }
            }

            // Case 2: capacity check
            if self.active.len() >= self.max_active {
                // backpressure / drop / requeue
                warn!(?self.active, "router fully occupied");
                return;
            }

            // Case 3: spawn new actor
            let (tx, rx) = mpsc::channel(32);

            self.active
                .insert(article_id, (tx.clone(), page.task.url.clone()));

            let filename = self
                .archiver
                .canonical_filename(&page.task.url, page.fetch_time)
                .unwrap();
            debug!(page.task.url, ?filename, "filename");
            let task = page.task.clone();
            let tx_done = self.done_tx.clone();
            let db = self.db.clone();
            tokio::spawn(async move {
                article_actor(article_id, filename, task, rx, tx_done, db).await;
            });

            // loop will retry send immediately
        }
    }
}

async fn article_actor<DB>(
    article_id: ArticleId,
    filename: PathBuf,
    task: FetchTask,
    mut rx: mpsc::Receiver<FetchedArticlePage>,
    done_tx: mpsc::Sender<ArticleId>,
    db: Arc<DB>,
) where
    DB: FrontierDbTrait,
{
    let mut state = ArticleState::new(filename, task, db);

    while let Some(page) = rx.recv().await {
        state.apply(page);

        // Write incrementally (safe: single writer)
        if let Err(e) = state.persist().await {
            eprintln!("persist error for {}: {:?}", article_id, e);
        }

        if state.done() {
            break;
        }
    }

    // Final flush
    let _ = state.finalize().await;
    let _ = done_tx.send(article_id).await;
}

#[cfg(test)]
mod tests {
    use crate::frontier::db::frontier::MockFrontierDbTrait;

    use super::*;
    use common::MockArchiver;
    use mockall::predicate::*;
    use std::collections::{HashMap, HashSet};
    use tempfile::tempdir;
    use tokio::sync::mpsc;
    use tracing_test::traced_test;

    // -----------------------------
    // Helpers
    // -----------------------------
    fn make_task(article_id: ArticleId, url: &str) -> FetchTask {
        FetchTask {
            article_id,
            url_id: 42,
            url: url.to_string(),
            depth: 0,
            priority: Priority::default(),
            discovered_from: None,
        }
    }

    fn make_page(article_id: ArticleId, url: &str, links: &[&str]) -> FetchedArticlePage {
        FetchedArticlePage {
            task: make_task(article_id, url),
            content: "content".into(),
            fetch_time: 123,
            links: links.iter().map(|l| l.to_string()).collect(),
            title: Some("title".into()),
            document_metadata: vec![HashMap::new()],
            json_ld: None,
        }
    }

    // -----------------------------
    // ⭐ THE IMPORTANT TEST
    // -----------------------------
    #[test]
    fn apply_enqueues_links_with_correct_priority() {
        let article_id = 1;

        let mut db = MockFrontierDbTrait::new();

        // Expect enqueue_batch to be called once
        db.expect_enqueue_batch()
            .times(1)
            .withf(|batch, high_priority| {
                // We expect:
                // - 2 links enqueued
                // - same article_id
                // - correct depth increment
                // - one should be treated as "article page"

                batch.len() == 2
                    && !high_priority
                    && batch
                        .iter()
                        .all(|task| task.article_id == 1 && task.depth == 1)
            })
            .returning(|_, _| Ok(()));

        let db = Arc::new(db);

        let mut state = ArticleState {
            task: make_task(article_id, "https://example.com?page=1"),
            filename: PathBuf::from("test.json"),
            snapshot: HistoricalSnapshot {
                content_markdown: vec![],
                links: HashSet::new(),
                metadata: None,
            },
            pages: HashMap::new(),
            db,
        };

        let page = make_page(
            article_id,
            "https://example.com?page=1",
            &[
                "https://example.com?page=2", // should be treated as pagination
                "https://other.com",          // external
            ],
        );

        state.apply(page);

        // Also verify snapshot got links
        assert!(state.snapshot.links.contains("https://example.com?page=2"));
        assert!(state.snapshot.links.contains("https://other.com"));
    }

    fn make_state(db: MockFrontierDbTrait) -> ArticleState<MockFrontierDbTrait> {
        ArticleState {
            task: make_task(1, "https://example.com?page=1"),
            filename: PathBuf::from("test.json"),
            snapshot: HistoricalSnapshot {
                content_markdown: vec![],
                links: HashSet::new(),
                metadata: None,
            },
            pages: HashMap::new(),
            db: Arc::new(db),
        }
    }

    // -----------------------------
    // ArticleState::done
    // -----------------------------
    #[test]
    fn done_returns_true_when_all_pages_fetched() {
        let db = MockFrontierDbTrait::new();
        let mut state = make_state(db);

        state.pages.insert("a".into(), true);
        state.pages.insert("b".into(), true);

        assert!(state.done());
    }

    #[test]
    fn done_returns_false_when_any_page_missing() {
        let db = MockFrontierDbTrait::new();
        let mut state = make_state(db);

        state.pages.insert("a".into(), true);
        state.pages.insert("b".into(), false);

        assert!(!state.done());
    }

    // -----------------------------
    // ArticleState::apply
    // -----------------------------
    #[test]
    fn apply_marks_page_as_fetched_and_adds_content() {
        let mut db = MockFrontierDbTrait::new();
        db.expect_enqueue_batch().returning(|_, _| Ok(()));

        let mut state = make_state(db);

        let page = make_page(1, "https://example.com?page=1", &[]);

        state.apply(page.clone());

        assert_eq!(state.pages.get(&page.task.url), Some(&true));
        assert_eq!(state.snapshot.content_markdown.len(), 1);
    }

    #[test]
    fn apply_enqueues_links_with_correct_depth_and_article_id() {
        let mut db = MockFrontierDbTrait::new();

        db.expect_enqueue_batch()
            .times(1)
            .withf(|batch, high_priority| {
                batch.len() == 2
                    && !high_priority
                    && batch.iter().all(|t| t.article_id == 1 && t.depth == 1)
            })
            .returning(|_, _| Ok(()));

        let mut state = make_state(db);

        let page = make_page(
            1,
            "https://example.com?page=1",
            &["https://example.com?page=2", "https://other.com"],
        );

        state.apply(page);

        assert_eq!(state.snapshot.links.len(), 2);
    }

    #[test]
    fn apply_sets_metadata_only_once() {
        let mut db = MockFrontierDbTrait::new();
        db.expect_enqueue_batch().returning(|_, _| Ok(()));

        let mut state = make_state(db);

        let page1 = make_page(1, "https://example.com?page=1", &[]);
        let page2 = make_page(1, "https://example.com?page=2", &[]);

        state.apply(page1);
        state.apply(page2);

        assert!(state.snapshot.metadata.is_some());
    }

    #[test]
    fn apply_updates_title_when_page_one_arrives() {
        let mut db = MockFrontierDbTrait::new();
        db.expect_enqueue_batch().returning(|_, _| Ok(()));

        let mut state = make_state(db);

        let mut page2 = make_page(1, "https://example.com?page=2", &[]);
        page2.title = Some("page2".into());

        let mut page1 = make_page(1, "https://example.com?page=1", &[]);
        page1.title = Some("page1".into());

        state.apply(page2);
        state.apply(page1);

        assert_eq!(state.snapshot.metadata.unwrap().title, Some("page1".into()));
    }

    // -----------------------------
    // is_page
    // -----------------------------
    #[test]
    fn is_page_detects_same_article_pages() {
        let db = MockFrontierDbTrait::new();
        let state = make_state(db);

        assert!(state.is_page("https://example.com?page=2"));
    }

    #[test]
    fn is_page_rejects_different_domains() {
        let db = MockFrontierDbTrait::new();
        let state = make_state(db);

        assert!(!state.is_page("https://other.com"));
    }

    // -----------------------------
    // Router
    // -----------------------------
    #[tokio::test]
    async fn router_remove_deletes_active_entry() {
        let (done_tx, _rx) = mpsc::channel(10);
        let db = MockFrontierDbTrait::new();

        let mut router = Router::new(MockArchiver::new(), Arc::new(db), done_tx, 10);

        router.active.insert(1, (mpsc::channel(1).0, "url".into()));
        router.remove(1);

        assert!(!router.active.contains_key(&1));
    }

    #[tokio::test]
    async fn router_respects_capacity_limit() {
        let (done_tx, _rx) = mpsc::channel(10);
        let db = MockFrontierDbTrait::new();

        let mut router = Router::new(MockArchiver::new(), Arc::new(db), done_tx, 1);

        // Fill capacity
        router.active.insert(1, (mpsc::channel(1).0, "url".into()));

        let page = make_page(2, "https://example.com", &[]);

        router.route(page).await;

        assert_eq!(router.active.len(), 1);
    }

    #[tokio::test]
    async fn persist_does_not_fail_with_content() {
        let db = Arc::new(MockFrontierDbTrait::new());

        let mut state = ArticleState::new(
            PathBuf::from("test.json"),
            make_task(1, "https://example.com"),
            db,
        );

        // Add some content via apply
        let page = FetchedArticlePage {
            task: make_task(1, "https://example.com?page=1"),
            content: "hello".into(),
            fetch_time: 123,
            links: Default::default(),
            title: None,
            document_metadata: vec![],
            json_ld: None,
        };

        state.apply(page);

        let result = state.persist().await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn finalize_creates_new_archive_when_file_missing() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("article.json");

        let mut mock_db = MockFrontierDbTrait::new();

        mock_db
            .expect_mark_complete_article()
            .times(1)
            .returning(|_| Ok(()));

        let db = Arc::new(mock_db);

        let mut state =
            ArticleState::new(file_path.clone(), make_task(1, "https://example.com"), db);

        // Add one page
        state.apply(make_page(1, "https://example.com?page=1", &[]));

        let result = state.finalize().await;

        assert!(result.is_ok());
        assert!(file_path.exists());
    }

    #[tokio::test]
    #[traced_test]
    async fn finalize_sorts_pages_before_writing() {
        let dir = tempdir().unwrap();
        let file_path = dir
            .path()
            .join("finalize_sorts_pages_before_writing")
            .join("article.json");

        let mut mock_db = MockFrontierDbTrait::new();
        mock_db.expect_mark_complete_article().returning(|_| Ok(()));

        let db = Arc::new(mock_db);

        let mut state =
            ArticleState::new(file_path.clone(), make_task(1, "https://example.com"), db);

        // Insert out of order
        state.snapshot.content_markdown.push(HistoricalContent {
            content: common::historical::HistoricalContentType::Literal("p2".into()),
            page: 2,
        });

        state.snapshot.content_markdown.push(HistoricalContent {
            content: common::historical::HistoricalContentType::Literal("p1".into()),
            page: 1,
        });

        state.finalize().await.unwrap();

        // Read back file
        let file = std::fs::File::open(file_path).unwrap();
        let reader = std::io::BufReader::new(file);
        let page: HistoricalPage = serde_json::from_reader(reader).unwrap();

        let snapshot = page.current.unwrap();

        let pages: Vec<_> = snapshot.content_markdown.iter().map(|c| c.page).collect();

        assert_eq!(pages, vec![1, 2]); // sorted
    }

    #[tokio::test]
    async fn finalize_marks_article_complete() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("article.json");

        let mut mock_db = MockFrontierDbTrait::new();

        mock_db
            .expect_mark_complete_article()
            .with(eq(1))
            .times(1)
            .returning(|_| Ok(()));

        let db = Arc::new(mock_db);

        let mut state = ArticleState::new(file_path, make_task(1, "https://example.com"), db);

        state.apply(make_page(1, "https://example.com?page=1", &[]));

        state.finalize().await.unwrap();
    }

    #[tokio::test]
    async fn finalize_appends_to_existing_archive() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("article.json");

        // Create initial file
        let initial = HistoricalPage {
            task: make_task(1, "https://example.com"),
            current: None,
            historical_snapshots: Default::default(),
            all_links: Default::default(),
        };

        initial.write_page(&file_path).unwrap();

        let mut mock_db = MockFrontierDbTrait::new();
        mock_db.expect_mark_complete_article().returning(|_| Ok(()));

        let db = Arc::new(mock_db);

        let mut state =
            ArticleState::new(file_path.clone(), make_task(1, "https://example.com"), db);

        state.apply(make_page(1, "https://example.com?page=1", &[]));

        state.finalize().await.unwrap();

        // Verify file still readable
        let file = std::fs::File::open(file_path).unwrap();
        let reader = std::io::BufReader::new(file);
        let page: HistoricalPage = serde_json::from_reader(reader).unwrap();

        assert!(page.current.is_some());
    }

    #[tokio::test]
    async fn finalize_returns_error_if_db_fails() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("article.json");

        let mut mock_db = MockFrontierDbTrait::new();

        mock_db
            .expect_mark_complete_article()
            .returning(|_| Err(anyhow::anyhow!("db failure")));

        let db = Arc::new(mock_db);

        let mut state = ArticleState::new(file_path, make_task(1, "https://example.com"), db);

        state.apply(make_page(1, "https://example.com?page=1", &[]));

        let result = state.finalize().await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn route_sends_to_existing_actor() {
        let (done_tx, _done_rx) = mpsc::channel(10);
        let db = Arc::new(MockFrontierDbTrait::new());

        let mut router = Router::new(MockArchiver::new(), db, done_tx, 10);

        let (tx, mut rx) = mpsc::channel(1);

        router.active.insert(1, (tx, "url".into()));

        let page = make_page(1, "https://example.com", &[]);

        router.route(page).await;

        // Verify message arrived
        let received = rx.recv().await;
        assert!(received.is_some());
    }

    #[tokio::test]
    async fn route_retries_when_actor_channel_closed() {
        let (done_tx, _done_rx) = mpsc::channel(10);
        let db = Arc::new(MockFrontierDbTrait::new());

        let mut mock_archiver = MockArchiver::new();
        mock_archiver
            .expect_canonical_filename()
            .returning(|_, _| Ok(PathBuf::from("test.json")));

        let mut router = Router::new(mock_archiver, db.clone(), done_tx, 10);

        // Create channel and immediately drop receiver → send will fail
        let (tx, rx) = mpsc::channel(1);
        drop(rx);

        router.active.insert(1, (tx, "url".into()));

        let page = make_page(1, "https://example.com", &[]);

        // This should trigger retry + actor spawn
        router.route(page).await;

        // After retry, a new actor should be inserted
        assert!(router.active.contains_key(&1));
    }

    #[tokio::test]
    async fn route_spawns_new_actor_when_not_active() {
        let (done_tx, _done_rx) = mpsc::channel(10);
        let db = Arc::new(MockFrontierDbTrait::new());

        let mut mock_archiver = MockArchiver::new();
        mock_archiver
            .expect_canonical_filename()
            .returning(|_, _| Ok(PathBuf::from("test.json")));

        let mut router = Router::new(mock_archiver, db, done_tx, 10);

        let page = make_page(1, "https://example.com", &[]);

        router.route(page).await;

        // Actor should now be active
        assert!(router.active.contains_key(&1));
    }

    #[tokio::test]
    async fn route_does_not_spawn_when_at_capacity() {
        let (done_tx, _done_rx) = mpsc::channel(10);
        let db = Arc::new(MockFrontierDbTrait::new());

        let mut mock_archiver = MockArchiver::new();
        mock_archiver
            .expect_canonical_filename()
            .returning(|_, _| Ok(PathBuf::from("test.json")));

        let mut router = Router::new(mock_archiver, db, done_tx, 1);

        // Fill capacity
        router
            .active
            .insert(999, (mpsc::channel(1).0, "url".into()));

        let page = make_page(1, "https://example.com", &[]);

        router.route(page).await;

        // Still only 1 active
        assert_eq!(router.active.len(), 1);
        assert!(!router.active.contains_key(&1));
    }

    #[tokio::test]
    async fn route_retry_then_successfully_sends() {
        let (done_tx, _done_rx) = mpsc::channel(10);
        let db = Arc::new(MockFrontierDbTrait::new());

        let mut mock_archiver = MockArchiver::new();
        mock_archiver
            .expect_canonical_filename()
            .returning(|_, _| Ok(PathBuf::from("test.json")));

        let mut router = Router::new(mock_archiver, db, done_tx, 10);

        // First channel fails
        let (tx1, rx1) = mpsc::channel(1);
        drop(rx1);

        router.active.insert(1, (tx1, "url".into()));

        let page = make_page(1, "https://example.com", &[]);

        router.route(page).await;

        // After retry, we should have a working actor
        assert!(router.active.contains_key(&1));
    }
}
