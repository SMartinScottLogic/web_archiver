use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::File,
    io::BufReader,
    path::PathBuf,
};

use anyhow::Context;
use common::{
    Archiver,
    historical::{HistoricalContent, HistoricalPage, HistoricalSnapshot},
    types::{ArticleId, FetchTask, PageMetadata, Priority},
    url::remove_pagination_params,
};
use reqwest::StatusCode;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::frontier::db::frontier::FrontierDb;

#[derive(Clone, Debug)]
pub struct Steve {
    pub task: FetchTask,
    pub content: String,
    pub fetch_time: i64,
    pub links: HashSet<String>,
}

pub struct Router<T: Archiver> {
    active: HashMap<ArticleId, (mpsc::Sender<Steve>, String)>,
    max_active: usize,
    archiver: T,
    done_tx: mpsc::Sender<ArticleId>,
    db: FrontierDb,
}

struct ArticleState {
    task: FetchTask,
    filename: PathBuf,
    snapshot: HistoricalSnapshot,
    pages: HashMap<String, bool>,
    db: FrontierDb,
}

impl ArticleState {
    fn new(filename: PathBuf, task: FetchTask, db: FrontierDb) -> Self {
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
        error!(known_remaining = ?self.pages.iter().filter(|page| !*page.1).collect::<Vec<_>>(), "known pages");
        self.pages.iter().all(|(_url, &fetched)| fetched)
    }

    fn apply(&mut self, page: Steve) {
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
                .enqueue_batch(&batch)
                .inspect_err(|e| error!("failed enqueuing {:?}", e));
        }

        let content = HistoricalContent {
            content: common::historical::HistoricalContentType::Literal(page.content.to_owned()),
            page: page_number,
        };
        self.snapshot.content_markdown.push(content);

        // Setup metadata
        if self.snapshot.metadata.is_none() {
            self.snapshot.metadata = Some(PageMetadata {
                status_code: StatusCode::OK.as_u16(),
                content_type: None,
                fetch_time: page.fetch_time.try_into().unwrap_or_default(),
                title: None,
                document_metadata: None,
            })
        }

        debug!(?page, ?self.snapshot, page_number, page_url = page.task.url, "apply");
        info!(?self.filename, "apply");
    }

    async fn persist(&self) -> Result<(), std::io::Error> {
        let pages = self
            .snapshot
            .content_markdown
            .iter()
            .map(|c| c.page)
            .collect::<Vec<_>>();
        info!(?self.filename, num_pages = self.snapshot.content_markdown.len(), ?pages, "persist");
        debug!(?self.filename, ?self.snapshot, "persist");
        Ok(())
    }

    async fn finalize(mut self) -> anyhow::Result<()> {
        info!(?self.filename, "finalize");
        debug!(?self.filename, ?self.snapshot, "finalize");
        // 1. Read from archive; or create empty record
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
        // 2. Add each page to record for current 'archival date'
        // 2a Sort by page
        self.snapshot
            .content_markdown
            .sort_by_cached_key(|page| page.page);
        // 2b Add to historical page
        historical_page.add_snapshot(self.snapshot);
        // 3. Save resultant file to archive (overwriting)
        historical_page.write_page(&self.filename)
        // 4. Update db
    }

    fn is_page(&self, url: &str) -> bool {
        remove_pagination_params(&self.task.url) == remove_pagination_params(url)
    }
}

impl<T: Archiver> Router<T> {
    pub fn new(
        archiver: T,
        db: FrontierDb,
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

    pub async fn route(&mut self, mut page: Steve) {
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

async fn article_actor(
    article_id: ArticleId,
    filename: PathBuf,
    task: FetchTask,
    mut rx: mpsc::Receiver<Steve>,
    done_tx: mpsc::Sender<ArticleId>,
    db: FrontierDb,
) {
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
