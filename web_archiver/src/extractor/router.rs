use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::BufReader,
    path::PathBuf,
    sync::Arc,
};

use common::{
    Archiver, JsonLd,
    historical::{HistoricalContent, HistoricalPage, HistoricalSnapshot},
    types::{ArticleId, FetchTask, PageMetadata, Priority},
    url::remove_pagination_params,
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
    active: HashMap<ArticleId, (mpsc::Sender<ArticleMessage>, String)>,
    max_active: usize,
    archiver: T,
    done_tx: mpsc::Sender<ArticleId>,
    db: Arc<DB>,
}

enum ArticleMessage {
    Fetched(FetchedArticlePage),
    Failed(FetchTask),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PageState {
    Pending,
    Fetched,
    Failed,
}

struct ArticleState<DB>
where
    DB: FrontierDbTrait,
{
    task: FetchTask,
    filename: PathBuf,
    snapshot: HistoricalSnapshot,
    pages: HashMap<String, PageState>,
    page_ids: HashMap<String, i64>,
    root_seen: bool,
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
            page_ids: HashMap::new(),
            root_seen: false,
            db,
        }
    }

    fn done(&self) -> bool {
        debug!(
            known_remaining = ?self.pages.iter().filter(|(_, state)| **state == PageState::Pending).collect::<Vec<_>>(),
            "known pages"
        );
        self.root_seen
            && self
                .pages
                .values()
                .all(|state| *state != PageState::Pending)
    }

    fn has_failed_page(&self) -> bool {
        self.pages.values().any(|state| *state == PageState::Failed)
    }

    fn apply(&mut self, page: FetchedArticlePage) -> bool {
        let url = page.task.url.clone();
        let is_root = is_article_root(&url);
        if (is_root && self.root_seen)
            || (!is_root && (!self.root_seen || self.pages.get(&url) != Some(&PageState::Pending)))
        {
            if let Err(err) = self.db.mark_url(page.task.url_id, "complete") {
                error!(?err, url = %url, "Failed to mark ignored page complete");
            }
            return false;
        }

        let page_number = match common::url::extract_page(&page.task.url) {
            common::url::Page::Number(page_number) => page_number,
            _ => 1,
        };

        self.root_seen |= is_root;
        self.pages.insert(url.clone(), PageState::Fetched);
        self.page_ids.insert(url, page.task.url_id);

        let mut batch = Vec::new();
        let mut newly_discovered_pages = Vec::new();
        // Add links to snapshot and article queue
        for link in &page.links {
            self.snapshot.links.insert(link.to_string());
            let mut priority = Priority::default();
            if self.is_page(link) {
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    self.pages.entry(link.to_string())
                {
                    newly_discovered_pages.push(link.to_string());
                    entry.insert(PageState::Pending);
                }
                priority = Priority::Article;
            }
            // TODO Set this from Hosts
            let use_playwright = false;
            batch.push(FetchTask {
                article_id: self.task.article_id,
                url_id: 0, // Will be set by DB
                url: link.to_string(),
                depth: self.task.depth + 1,
                priority,
                discovered_from: Some(self.task.url_id),
                use_playwright,
            });
        }
        if !batch.is_empty() {
            match self.db.enqueue_batch(&batch, false) {
                Err(err) => {
                    error!(?err, "failed enqueuing discovered links");
                    for url in newly_discovered_pages {
                        self.pages.insert(url, PageState::Failed);
                    }
                }
                Ok(()) => {
                    for url in newly_discovered_pages {
                        match self.db.url_is_skipped(&url) {
                            Ok(true) => {
                                self.pages.insert(url, PageState::Failed);
                            }
                            Ok(false) => {}
                            Err(err) => {
                                error!(?err, %url, "Failed to check pagination URL status");
                                self.pages.insert(url, PageState::Failed);
                            }
                        }
                    }
                }
            }
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
                    authors: page
                        .json_ld
                        .as_ref()
                        .map(|j| j.authors())
                        .unwrap_or_default(),
                    title: page.title.clone(),
                    document_metadata: Some(page.document_metadata.clone()),
                    json_ld: page.json_ld.clone(),
                });
            }
            Some(metadata) if page_number == 1 => {
                metadata.title = page.title.clone();
                metadata.json_ld = page.json_ld.clone();
                metadata.authors = page
                    .json_ld
                    .as_ref()
                    .map(|json_ld| json_ld.authors())
                    .unwrap_or_default()
            }
            Some(_metadata) => {}
        };

        // Replace task if page == 1
        if page_number == 1 {
            self.task = page.task.clone()
        }

        debug!(?self.filename, ?page, ?self.snapshot, page_number, page_url = page.task.url, "apply");
        true
    }

    fn fail_page(&mut self, task: &FetchTask) {
        if self.pages.get(&task.url) == Some(&PageState::Pending) {
            self.pages.insert(task.url.clone(), PageState::Failed);
            self.page_ids.insert(task.url.clone(), task.url_id);
        }
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
        if !self.done() {
            anyhow::bail!("cannot finalize an article while discovered pages are unfetched");
        }
        if self.has_failed_page() {
            anyhow::bail!("cannot finalize an article with a terminally failed page");
        }
        if event_enabled!(Level::DEBUG) {
            debug!(?self.filename, ?self.snapshot, "finalize");
        } else {
            debug!(?self.filename, "finalize");
        }
        let article_id = self.task.article_id;
        // 1. Read from archive, or create empty record
        let (mut historical_page, new_file) = self.read_or_default();

        historical_page.task = self.task;
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
        let r = self.db.mark_article(article_id, "complete");
        info!(?self.filename, new_file, "finalized");
        r
    }

    fn is_page(&self, url: &str) -> bool {
        remove_pagination_params(&self.task.url) == remove_pagination_params(url)
    }

    fn read_or_default(&self) -> (HistoricalPage, bool) {
        if std::fs::exists(&self.filename)
            .inspect_err(|e| error!("Cannot test for existence {:?}: {:?}", self.filename, e))
            .unwrap()
        {
            // File exists
            let file = File::open(&self.filename)
                .inspect_err(|e| error!("Cannot open {:?}: {:?}", self.filename, e))
                .unwrap();
            let reader = BufReader::new(file);
            let historical_page = serde_json::from_reader(reader)
                .inspect_err(|e| error!("Failed to read from {:?}: {:?}", self.filename, e))
                .unwrap();
            (historical_page, false)
        } else {
            (HistoricalPage::new(self.task.clone()), true)
        }
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
        if !self.active.contains_key(&article_id) && !is_article_root(&page.task.url) {
            if let Err(err) = self.db.mark_url(page.task.url_id, "complete") {
                error!(?err, url = %page.task.url, "Failed to mark unlinked page complete");
            }
            return;
        }
        loop {
            // Case 1: already active
            if let Some((tx, _)) = self.active.get(&article_id) {
                match tx.send(ArticleMessage::Fetched(page)).await {
                    Ok(_) => return,
                    Err(e) => {
                        // actor died → recover page and retry
                        let ArticleMessage::Fetched(recovered_page) = e.0 else {
                            return;
                        };
                        page = recovered_page;
                        self.active.remove(&article_id);
                        if !is_article_root(&page.task.url) {
                            if let Err(err) = self.db.mark_url(page.task.url_id, "complete") {
                                error!(?err, url = %page.task.url, "Failed to mark unlinked page complete");
                            }
                            return;
                        }
                        continue;
                    }
                }
            }

            // Case 2: capacity check
            if self.active.len() >= self.max_active {
                // backpressure / drop / requeue
                warn!(?self.active, "router fully occupied");
                if let Err(e) = self.db.fail_url(page.task.url_id, true) {
                    error!(
                        ?e,
                        url = %page.task.url,
                        "Failed to mark unrouted article page retryable"
                    );
                }
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

    pub async fn route_failure(&mut self, task: FetchTask) {
        if let Some((tx, _)) = self.active.get(&task.article_id) {
            if tx.send(ArticleMessage::Failed(task.clone())).await.is_err() {
                warn!(url = %task.url, "Article actor closed before terminal fetch failure arrived");
            }
        }
    }
}

fn is_article_root(url: &str) -> bool {
    match common::url::extract_page(url) {
        common::url::Page::Number(1) | common::url::Page::None | common::url::Page::Text(_) => true,
        common::url::Page::Number(_) => false,
    }
}

async fn article_actor<DB>(
    article_id: ArticleId,
    filename: PathBuf,
    task: FetchTask,
    mut rx: mpsc::Receiver<ArticleMessage>,
    done_tx: mpsc::Sender<ArticleId>,
    db: Arc<DB>,
) where
    DB: FrontierDbTrait,
{
    let mut state = ArticleState::new(filename, task, db);

    while let Some(message) = rx.recv().await {
        match message {
            ArticleMessage::Fetched(page) => {
                if state.apply(page) {
                    if let Err(e) = state.persist().await {
                        error!(?e, article_id, "Failed to persist article state");
                    }
                }
            }
            ArticleMessage::Failed(task) => state.fail_page(&task),
        }

        if state.done() {
            break;
        }
    }

    if state.done() && !state.has_failed_page() {
        if let Err(e) = state.finalize().await {
            error!(?e, article_id, "Failed to finalize article");
        }
    } else {
        warn!(article_id, "Discarding incomplete article snapshot");
        for (url, page_state) in &state.pages {
            if *page_state == PageState::Fetched {
                if let Some(url_id) = state.page_ids.get(url) {
                    if let Err(err) = state.db.mark_url(*url_id, "complete") {
                        error!(?err, %url, "Failed to mark fetched page complete after discarding article");
                    }
                }
            }
        }
    }
    let _ = done_tx.send(article_id).await;
}

#[cfg(test)]
mod tests {
    use crate::frontier::db::frontier::{FrontierDb, MockFrontierDbTrait};

    use super::*;
    use common::MockArchiver;
    use mockall::predicate::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;
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
            use_playwright: false,
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

    fn make_archived_page(task: FetchTask, links: &[String]) -> FetchedArticlePage {
        FetchedArticlePage {
            task,
            content: "new content".into(),
            fetch_time: 123,
            links: links.iter().cloned().collect(),
            title: Some("title".into()),
            document_metadata: vec![HashMap::new()],
            json_ld: None,
        }
    }

    fn seed_completed_ten_page_article() -> (Arc<FrontierDb>, Vec<FetchTask>) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::frontier::db::schema::init_schema(&conn).unwrap();
        let db = Arc::new(FrontierDb {
            conn: Arc::new(Mutex::new(conn)),
        });

        let urls = (1..=10)
            .map(|page| format!("https://refresh-tests.example.invalid/article?page={page}"))
            .collect::<Vec<_>>();
        let seeds = urls
            .iter()
            .map(|url| FetchTask {
                article_id: 0,
                url_id: 0,
                url: url.clone(),
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
                use_playwright: false,
            })
            .collect::<Vec<_>>();
        db.enqueue_batch(&seeds, false).unwrap();

        let tasks = {
            let conn = db.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT urls.id, urls.article_id, urls.url, frontier.depth,
                            frontier.priority, frontier.discovered_from, urls.use_playwright
                     FROM urls JOIN frontier ON frontier.url_id = urls.id
                     ORDER BY CAST(substr(urls.url, instr(urls.url, '=') + 1) AS INTEGER)",
                )
                .unwrap();
            stmt.query_map([], |row| {
                Ok(FetchTask {
                    url_id: row.get(0)?,
                    article_id: row.get(1)?,
                    url: row.get(2)?,
                    depth: row.get(3)?,
                    priority: row.get(4)?,
                    discovered_from: row.get(5)?,
                    use_playwright: row.get(6)?,
                })
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
        };

        for task in &tasks {
            db.mark(task.url_id, "complete").unwrap();
        }
        (db, tasks)
    }

    fn write_previous_ten_page_archive(path: &std::path::Path, task: FetchTask) {
        let mut historical = HistoricalPage::new(task);
        historical.current = Some(HistoricalSnapshot {
            content_markdown: (1..=10)
                .map(|page| HistoricalContent {
                    content: common::historical::HistoricalContentType::Literal(format!(
                        "previous page {page}"
                    )),
                    page,
                })
                .collect(),
            links: HashSet::new(),
            metadata: None,
        });
        historical.write_page(path).unwrap();
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
        db.expect_url_is_skipped().returning(|_| Ok(false));

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
            page_ids: HashMap::new(),
            root_seen: false,
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
        let mut db = db;
        db.expect_url_is_skipped().returning(|_| Ok(false));
        ArticleState {
            task: make_task(1, "https://example.com?page=1"),
            filename: PathBuf::from("test.json"),
            snapshot: HistoricalSnapshot {
                content_markdown: vec![],
                links: HashSet::new(),
                metadata: None,
            },
            pages: HashMap::new(),
            page_ids: HashMap::new(),
            root_seen: false,
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

        state.root_seen = true;
        state.pages.insert("a".into(), PageState::Fetched);
        state.pages.insert("b".into(), PageState::Fetched);

        assert!(state.done());
    }

    #[test]
    fn done_returns_false_when_any_page_missing() {
        let db = MockFrontierDbTrait::new();
        let mut state = make_state(db);

        state.root_seen = true;
        state.pages.insert("a".into(), PageState::Fetched);
        state.pages.insert("b".into(), PageState::Pending);

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

        assert_eq!(state.pages.get(&page.task.url), Some(&PageState::Fetched));
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

    #[tokio::test]
    async fn newly_discovered_article_pages_are_fetched_before_finalization() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("article.json");
        let mut db = MockFrontierDbTrait::new();
        db.expect_enqueue_batch().returning(|_, _| Ok(()));
        db.expect_url_is_skipped().returning(|_| Ok(false));
        db.expect_mark_article().returning(|_, _| Ok(()));
        let mut state = ArticleState::new(
            file_path.clone(),
            make_task(1, "https://example.com?page=1"),
            Arc::new(db),
        );

        state.apply(make_page(
            1,
            "https://example.com?page=1",
            &["https://example.com?page=2"],
        ));
        assert_eq!(
            state.pages.get("https://example.com?page=2"),
            Some(&PageState::Pending)
        );
        assert!(!state.done());
        assert!(!file_path.exists());

        state.apply(make_page(1, "https://example.com?page=2", &[]));
        assert!(state.done());
        state.finalize().await.unwrap();

        let file = std::fs::File::open(file_path).unwrap();
        let archived: HistoricalPage =
            serde_json::from_reader(std::io::BufReader::new(file)).unwrap();
        let page_numbers = archived
            .current
            .unwrap()
            .content_markdown
            .into_iter()
            .map(|content| content.page)
            .collect::<Vec<_>>();
        assert_eq!(page_numbers, vec![1, 2]);
    }

    #[test]
    fn apply_sets_metadata_only_once() {
        let mut db = MockFrontierDbTrait::new();
        db.expect_enqueue_batch().returning(|_, _| Ok(()));

        let mut state = make_state(db);

        let mut page1 = make_page(1, "https://example.com?page=1", &[]);
        page1.links.insert("https://example.com?page=2".into());
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
        page1.links.insert("https://example.com?page=2".into());

        state.apply(page1);
        state.apply(page2);

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
        let mut db = MockFrontierDbTrait::new();
        db.expect_fail_url()
            .withf(|url_id, retryable| *url_id == 42 && *retryable)
            .times(1)
            .returning(|_, _| Ok(false));

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
            .expect_mark_article()
            .times(1)
            .returning(|_, _| Ok(()));

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
        mock_db.expect_mark_article().returning(|_, _| Ok(()));

        let db = Arc::new(mock_db);

        let mut state =
            ArticleState::new(file_path.clone(), make_task(1, "https://example.com"), db);
        state
            .pages
            .insert("https://example.com?page=1".into(), PageState::Fetched);
        state.root_seen = true;

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
    async fn finalize_does_not_replace_archive_with_incomplete_snapshot() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("article.json");
        let previous_archive = "previous complete archive";
        std::fs::write(&file_path, previous_archive).unwrap();

        let mut mock_db = MockFrontierDbTrait::new();
        mock_db.expect_mark_article().times(0);
        let mut state = ArticleState::new(
            file_path.clone(),
            make_task(1, "https://example.com?page=1"),
            Arc::new(mock_db),
        );
        state
            .pages
            .insert("https://example.com?page=1".into(), PageState::Fetched);
        state
            .pages
            .insert("https://example.com?page=2".into(), PageState::Pending);
        state.root_seen = true;

        assert!(state.finalize().await.is_err());
        assert_eq!(
            std::fs::read_to_string(file_path).unwrap(),
            previous_archive
        );
    }

    #[test]
    fn terminal_page_failure_resolves_known_page_without_completing_article() {
        let mut db = MockFrontierDbTrait::new();
        db.expect_enqueue_batch().returning(|_, _| Ok(()));
        let mut state = make_state(db);
        state.apply(make_page(
            1,
            "https://example.com?page=1",
            &["https://example.com?page=2"],
        ));

        let failed_page = make_task(1, "https://example.com?page=2");
        state.fail_page(&failed_page);

        assert!(state.done());
        assert!(state.has_failed_page());
    }

    #[test]
    fn excluded_pagination_page_resolves_as_terminal_for_article_assembly() {
        let mut db = MockFrontierDbTrait::new();
        db.expect_enqueue_batch().returning(|_, _| Ok(()));
        db.expect_url_is_skipped().returning(|_| Ok(true));
        let mut state = ArticleState::new(
            PathBuf::from("test.json"),
            make_task(1, "https://example.com?page=1"),
            Arc::new(db),
        );

        state.apply(make_page(
            1,
            "https://example.com?page=1",
            &["https://example.com?page=2"],
        ));

        assert!(state.done());
        assert!(state.has_failed_page());
    }

    #[tokio::test]
    async fn article_actor_discards_snapshot_and_signals_completion_after_terminal_page_failure() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("article.json");
        let previous_archive = "previous complete archive";
        std::fs::write(&file_path, previous_archive).unwrap();

        let mut db = MockFrontierDbTrait::new();
        db.expect_enqueue_batch().returning(|_, _| Ok(()));
        db.expect_url_is_skipped().returning(|_| Ok(false));
        db.expect_mark_article().times(0);
        db.expect_mark_url()
            .withf(|url_id, status| *url_id == 42 && status == "complete")
            .times(1)
            .returning(|_, _| Ok(()));

        let (done_tx, mut done_rx) = mpsc::channel(1);
        let (tx, rx) = mpsc::channel(2);
        let task = make_task(1, "https://example.com?page=1");
        let actor = tokio::spawn(article_actor(
            1,
            file_path.clone(),
            task,
            rx,
            done_tx,
            Arc::new(db),
        ));

        tx.send(ArticleMessage::Fetched(make_page(
            1,
            "https://example.com?page=1",
            &["https://example.com?page=2"],
        )))
        .await
        .unwrap();
        let mut failed_page = make_task(1, "https://example.com?page=2");
        failed_page.url_id = 43;
        tx.send(ArticleMessage::Failed(failed_page)).await.unwrap();

        assert_eq!(done_rx.recv().await, Some(1));
        actor.await.unwrap();
        assert_eq!(
            std::fs::read_to_string(file_path).unwrap(),
            previous_archive
        );
    }

    #[test]
    fn unlinked_stale_page_does_not_join_current_snapshot() {
        let mut db = MockFrontierDbTrait::new();
        db.expect_mark_url()
            .withf(|url_id, status| *url_id == 42 && status == "complete")
            .times(1)
            .returning(|_, _| Ok(()));
        db.expect_enqueue_batch().returning(|_, _| Ok(()));

        let mut state = make_state(db);
        state.apply(make_page(1, "https://example.com?page=1", &[]));

        let accepted = state.apply(make_page(1, "https://example.com?page=2", &[]));

        assert!(!accepted);
        assert_eq!(state.snapshot.content_markdown.len(), 1);
        assert!(!state.pages.contains_key("https://example.com?page=2"));
    }

    #[test]
    fn complete_refresh_replaces_current_page_set_when_page_count_shrinks() {
        let task = make_task(1, "https://example.com?page=1");
        let mut archived = HistoricalPage::new(task);
        archived.current = Some(HistoricalSnapshot {
            content_markdown: (1..=3)
                .map(|page| HistoricalContent {
                    content: common::historical::HistoricalContentType::Literal(format!(
                        "old page {page}"
                    )),
                    page,
                })
                .collect(),
            links: HashSet::new(),
            metadata: None,
        });

        archived
            .add_snapshot(HistoricalSnapshot {
                content_markdown: (1..=2)
                    .map(|page| HistoricalContent {
                        content: common::historical::HistoricalContentType::Literal(format!(
                            "new page {page}"
                        )),
                        page,
                    })
                    .collect(),
                links: HashSet::new(),
                metadata: None,
            })
            .unwrap();

        let current_pages = archived
            .current
            .as_ref()
            .unwrap()
            .content_markdown
            .iter()
            .map(|content| content.page)
            .collect::<Vec<_>>();
        assert_eq!(current_pages, vec![1, 2]);
        assert_eq!(archived.historical_snapshots.len(), 1);
    }

    #[tokio::test]
    async fn refetching_page_ten_and_failing_does_not_write_partial_archive() {
        let dir = tempdir().unwrap();
        let archive_path = dir.path().join("article.json");
        let (db, tasks) = seed_completed_ten_page_article();
        write_previous_ten_page_archive(&archive_path, tasks[0].clone());
        let original_archive = std::fs::read(&archive_path).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            let now = chrono::Utc::now().timestamp();
            conn.execute("UPDATE frontier SET latest_fetch_time = ?1", [now])
                .unwrap();
            conn.execute(
                "UPDATE frontier SET latest_fetch_time = 0 WHERE url_id = ?1",
                [tasks[9].url_id],
            )
            .unwrap();
        }
        assert_eq!(db.requeue_stale_or_failed(1).unwrap(), 1);

        let mut state = ArticleState::new(archive_path.clone(), tasks[0].clone(), db.clone());

        for page_number in 1..=9 {
            let task = if page_number == 1 {
                tasks[0].clone()
            } else {
                let claimed = db.claim_next(1).unwrap().pop().unwrap();
                assert_eq!(claimed.url, tasks[page_number - 1].url);
                claimed
            };
            let links = if page_number < 10 {
                vec![tasks[page_number].url.clone()]
            } else {
                Vec::new()
            };
            state.apply(make_archived_page(task, &links));
        }

        let failed_page = db.claim_next(1).unwrap().pop().unwrap();
        assert_eq!(failed_page.url, tasks[9].url);
        assert!(db.fail(failed_page.url_id, false).unwrap());
        state.fail_page(&failed_page);

        assert!(state.done());
        assert!(state.has_failed_page());
        assert!(state.finalize().await.is_err());
        assert_eq!(std::fs::read(&archive_path).unwrap(), original_archive);
    }

    #[tokio::test]
    async fn refetching_page_six_writes_only_linked_pages_one_through_eight() {
        let dir = tempdir().unwrap();
        let archive_path = dir.path().join("article.json");
        let (db, tasks) = seed_completed_ten_page_article();
        write_previous_ten_page_archive(&archive_path, tasks[0].clone());

        {
            let conn = db.conn.lock().unwrap();
            let now = chrono::Utc::now().timestamp();
            conn.execute("UPDATE frontier SET latest_fetch_time = ?1", [now])
                .unwrap();
            conn.execute(
                "UPDATE frontier SET latest_fetch_time = 0 WHERE url_id = ?1",
                [tasks[5].url_id],
            )
            .unwrap();
        }
        assert_eq!(db.requeue_stale_or_failed(1).unwrap(), 1);

        let mut state = ArticleState::new(archive_path.clone(), tasks[0].clone(), db.clone());
        let mut fetched_pages = Vec::new();

        state.apply(make_archived_page(
            tasks[0].clone(),
            &[tasks[1].url.clone()],
        ));
        fetched_pages.push(tasks[0].url.clone());

        for page_number in 2..=8 {
            let task = db.claim_next(1).unwrap().pop().unwrap();
            assert_eq!(task.url, tasks[page_number - 1].url);
            fetched_pages.push(task.url.clone());
            let links = if page_number < 8 {
                vec![tasks[page_number].url.clone()]
            } else {
                Vec::new()
            };
            state.apply(make_archived_page(task, &links));
        }

        assert_eq!(
            fetched_pages,
            tasks[..8]
                .iter()
                .map(|task| task.url.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(db.claim_next(1).unwrap().len(), 0);

        for task in &tasks[8..] {
            let (status, claimed_at): (String, Option<i64>) = db
                .conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT status, claimed_at FROM frontier WHERE url_id = ?1",
                    [task.url_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(status, "complete", "unexpected refetch of {}", task.url);
            assert_eq!(claimed_at, None, "page was attempted: {}", task.url);
        }

        assert!(state.done());
        state.finalize().await.unwrap();

        let file = File::open(&archive_path).unwrap();
        let archived: HistoricalPage = serde_json::from_reader(BufReader::new(file)).unwrap();
        let current_pages = archived
            .current
            .unwrap()
            .content_markdown
            .into_iter()
            .map(|content| content.page)
            .collect::<Vec<_>>();
        assert_eq!(current_pages, (1..=8).collect::<Vec<_>>());
        assert_eq!(archived.historical_snapshots.len(), 1);
    }

    #[tokio::test]
    async fn finalize_marks_article_complete() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("article.json");

        let mut mock_db = MockFrontierDbTrait::new();

        mock_db
            .expect_mark_article()
            .with(eq(1), eq("complete"))
            .times(1)
            .returning(|_, _| Ok(()));

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
        let initial = HistoricalPage::new(make_task(1, "https://example.com"));

        initial.write_page(&file_path).unwrap();

        let mut mock_db = MockFrontierDbTrait::new();
        mock_db.expect_mark_article().returning(|_, _| Ok(()));

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
            .expect_mark_article()
            .returning(|_, _| Err(anyhow::anyhow!("db failure")));

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
    async fn route_forwards_terminal_failures_to_active_article_actor() {
        let (done_tx, _done_rx) = mpsc::channel(10);
        let db = Arc::new(MockFrontierDbTrait::new());
        let mut router = Router::new(MockArchiver::new(), db, done_tx, 10);
        let (tx, mut rx) = mpsc::channel(1);
        router.active.insert(1, (tx, "url".into()));

        let task = make_task(1, "https://example.com?page=2");
        router.route_failure(task.clone()).await;

        assert!(matches!(
            rx.recv().await,
            Some(ArticleMessage::Failed(failed)) if failed.url_id == task.url_id
        ));
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
    async fn route_does_not_start_snapshot_for_unlinked_pagination_page() {
        let (done_tx, _done_rx) = mpsc::channel(10);
        let mut db = MockFrontierDbTrait::new();
        db.expect_mark_url()
            .withf(|url_id, status| *url_id == 42 && status == "complete")
            .times(1)
            .returning(|_, _| Ok(()));
        let mut router = Router::new(MockArchiver::new(), Arc::new(db), done_tx, 10);

        router
            .route(make_page(1, "https://example.com?page=3", &[]))
            .await;

        assert!(router.active.is_empty());
    }

    #[tokio::test]
    async fn route_does_not_spawn_when_at_capacity() {
        let (done_tx, _done_rx) = mpsc::channel(10);
        let mut mock_db = MockFrontierDbTrait::new();
        mock_db
            .expect_fail_url()
            .withf(|url_id, retryable| *url_id == 42 && *retryable)
            .times(1)
            .returning(|_, _| Ok(false));
        let db = Arc::new(mock_db);

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
