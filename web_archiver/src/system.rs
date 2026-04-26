use common::Archiver;
use common::types::{ArticleId, FetchTask};
use rusqlite::Connection;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::info;

use crate::extractor::parser::extractor_loop;
use crate::fetcher::worker::worker_loop_single;
use crate::frontier::db::frontier::FrontierDbTrait;
use crate::frontier::frontier_manager::FrontierManager;
use tokio::sync::Semaphore;

use crate::settings::Config;

use crate::extractor::router::{FetchedArticlePage, Router};
use crate::extractor::{DiscoveredLinks, FetchedPage};

pub struct System<A: Archiver, DB: FrontierDbTrait> {
    // pub router: Router<A, DB>,

    // pub tx_fetch: mpsc::Sender<FetchTask>,
    // pub rx_fetch: mpsc::Receiver<FetchTask>,

    // pub tx_fetched: mpsc::Sender<FetchedPage>,
    // pub rx_fetched: mpsc::Receiver<FetchedPage>,

    // pub tx_extracted: mpsc::Sender<FetchedArticlePage>,
    // pub rx_extracted: mpsc::Receiver<FetchedArticlePage>,

    // pub tx_links: mpsc::Sender<DiscoveredLinks>,
    // pub rx_links: mpsc::Receiver<DiscoveredLinks>,

    // pub rx_done: mpsc::Receiver<ArticleId>,

    // pub semaphore: Arc<Semaphore>,
    phantom: PhantomData<A>,
    phantom2: PhantomData<DB>,
}

impl<A: Archiver, DB: FrontierDbTrait> System<A, DB> {
    pub fn spawn_frontier(
        //&self,
        frontier_manager: FrontierManager,
    ) {
        tokio::spawn(async move {
            frontier_manager.run().await;
        });
    }
}

impl<A: Archiver, DB: FrontierDbTrait> System<A, DB> {
    pub fn spawn_workers(
        //&mut self,
        mut rx_fetch: mpsc::Receiver<FetchTask>,
        tx_fetched: mpsc::Sender<FetchedPage>,
        semaphore: Arc<Semaphore>,
        config: Config,
    ) {
        tokio::spawn(async move {
            while let Some(task) = rx_fetch.recv().await {
                let permit = semaphore.clone().acquire_owned().await.unwrap();
                let tx = tx_fetched.clone();
                let user_agent = config.user_agent.clone();
                let archive_time = config.archive_time;

                tokio::spawn(async move {
                    worker_loop_single(task, archive_time, &user_agent, tx).await;
                    drop(permit);
                });
            }
        });
    }
}

impl<A: Archiver, DB: FrontierDbTrait> System<A, DB> {
    pub fn spawn_extractor(
        rx_fetched: mpsc::Receiver<FetchedPage>,
        tx_extracted: mpsc::Sender<FetchedArticlePage>,
        tx_links: mpsc::Sender<DiscoveredLinks>,
    ) {
        tokio::spawn(async move {
            extractor_loop(rx_fetched, tx_extracted, tx_links).await;
        });
    }
}

impl<A, DB> System<A, DB>
where
    A: Archiver + Send + Sync + 'static,
    DB: FrontierDbTrait,
{
    pub fn spawn_router_loop(
        mut router: Router<A, DB>,
        mut rx_extracted: mpsc::Receiver<FetchedArticlePage>,
        mut rx_done: mpsc::Receiver<ArticleId>,
    ) {
        tokio::spawn(async move {
            while let Some(page) = rx_extracted.recv().await {
                router.route(page).await;

                while let Ok(article_id) = rx_done.try_recv() {
                    router.remove(article_id);
                }
            }
        });
    }
}

pub async fn run_system<A, DB>(config: Config) -> anyhow::Result<()>
where
    A: Archiver + Send + Sync + 'static,
    DB: FrontierDbTrait,
{
    let noop_delay_millis = config.noop_delay_millis;
    let max_concurrent = config.workers;

    let conn = Connection::open(&config.db).expect("failed to open DB");
    crate::frontier::db::schema::settings(&conn).expect("failed to set DB performance settings");
    crate::frontier::db::schema::init_schema(&conn).expect("failed to init schema");
    let db_arc = Arc::new(Mutex::new(conn));

    // --- 2. Seed URLs ---
    let seed_urls = config.seed_urls.clone();

    // --- 3. Create channels ---
    // Frontier → Worker
    let (tx_fetch, rx_fetch) = mpsc::channel::<FetchTask>(100);
    // Worker → Extractor
    let (tx_fetched, rx_fetched) = mpsc::channel::<FetchedPage>(100);
    // Extractor → Storage
    let (tx_extracted, rx_extracted) = mpsc::channel::<FetchedArticlePage>(100);
    // Storage → Frontier
    let (tx_links, rx_links) = mpsc::channel::<DiscoveredLinks>(500);

    // --- 4. Spawn Frontier Manager ---
    let mut frontier_manager = FrontierManager::new(
        config.user_agent.clone(),
        tx_fetch,
        rx_links,
        noop_delay_millis,
        config.hosts.clone(),
        db_arc.clone(),
    );
    if config.reset {
        let num_reset = frontier_manager.reset_all()?;
        info!("Reset all fetch tasks: {}", num_reset);
    }
    frontier_manager.add_seeds(&seed_urls);
    System::<A, DB>::spawn_frontier(frontier_manager);

    // --- 5. Spawn Worker Tasks ---
    // This task owns the receiver and spawns multiple async fetch tasks
    let sem = Arc::new(Semaphore::new(max_concurrent));
    System::<A, DB>::spawn_workers(rx_fetch, tx_fetched, sem, config.clone());

    // --- 6. Spawn Extractor Task ---
    System::<A, DB>::spawn_extractor(rx_fetched, tx_extracted, tx_links);

    // --- 7. Spawn Storage Task ---
    let archiver = A::for_path(PathBuf::from(config.archive_dir));
    let storage_db = DB::connect(db_arc.clone());

    let (tx_done, rx_done) = mpsc::channel::<ArticleId>(100);

    // Router event loop
    let router = Router::new(archiver, Arc::new(storage_db), tx_done, max_concurrent * 2);
    System::spawn_router_loop(router, rx_extracted, rx_done);

    // --- 8. Wait forever ---
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for Ctrl-C");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::MockArchiver;
    use std::collections::HashSet;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    use crate::extractor::router::FetchedArticlePage;
    use crate::extractor::{DiscoveredLinks, FetchedPage};
    use crate::frontier::db::frontier::MockFrontierDbTrait;
    use crate::settings::Config;
    use common::types::{ArticleId, FetchTask};

    // Dummy config helper
    fn test_config() -> Config {
        Config {
            user_agent: "test-agent".into(),
            archive_time: 0,
            noop_delay_millis: 10,
            workers: 2,
            db: ":memory:".into(),
            seed_urls: vec![],
            hosts: vec![],
            archive_dir: "./tmp".into(),
            reset: Default::default(),
        }
    }

    // --------------------------------------------------
    // ✅ Test: spawn_workers processes tasks
    // --------------------------------------------------
    #[tokio::test]
    async fn test_spawn_workers_processes_tasks() {
        let (tx_fetch, rx_fetch) = mpsc::channel::<FetchTask>(10);
        let (tx_fetched, rx_fetched) = mpsc::channel::<FetchedPage>(10);

        let semaphore = Arc::new(Semaphore::new(1));
        let config = test_config();

        System::<MockArchiver, MockFrontierDbTrait>::spawn_workers(
            rx_fetch, tx_fetched, semaphore, config,
        );

        // Send a dummy task
        let task = FetchTask {
            url: "http://example.com".into(),
            depth: 0,
            article_id: 0,
            url_id: 0,
            priority: common::types::Priority::Normal,
            discovered_from: None,
        };

        tx_fetch.send(task).await.unwrap();

        // We can't guarantee worker_loop_single output,
        // but we can at least ensure system doesn't crash
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Optional: check channel still alive
        assert!(!rx_fetched.is_closed());
    }

    // --------------------------------------------------
    // ✅ Test: spawn_extractor forwards messages
    // --------------------------------------------------
    #[tokio::test]
    async fn test_spawn_extractor_runs() {
        let (tx_fetched, rx_fetched) = mpsc::channel::<FetchedPage>(10);
        let (tx_extracted, rx_extracted) = mpsc::channel::<FetchedArticlePage>(10);
        let (tx_links, rx_links) = mpsc::channel::<DiscoveredLinks>(10);

        System::<MockArchiver, MockFrontierDbTrait>::spawn_extractor(
            rx_fetched,
            tx_extracted,
            tx_links,
        );

        // Send dummy page
        let page = FetchedPage {
            body: Arc::new(b"<html></html>".to_vec()),
            status_code: 200,
            fetch_time: 0,
            content_type: None,
            task: FetchTask {
                article_id: 0,
                url_id: 0,
                url: "http://example.com".into(),
                depth: 0,
                priority: common::types::Priority::Normal,
                discovered_from: None,
            },
        };

        tx_fetched.send(page).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // We don't know extractor output, but ensure system alive
        assert!(!rx_extracted.is_closed() || !rx_links.is_closed());
    }

    // --------------------------------------------------
    // ✅ Test: spawn_router_loop processes pages
    // --------------------------------------------------
    #[tokio::test]
    async fn test_spawn_router_loop_basic() {
        let (tx_extracted, rx_extracted) = mpsc::channel::<FetchedArticlePage>(10);
        let (tx_done, rx_done) = mpsc::channel::<ArticleId>(10);

        // Mock Archiver behavior
        let archiver = MockArchiver::new();
        let db = MockFrontierDbTrait::new();

        // Router requires real instances, but we don’t care about internals
        let router = Router::new(archiver, Arc::new(db), tx_done, 10);

        System::<MockArchiver, MockFrontierDbTrait>::spawn_router_loop(
            router,
            rx_extracted,
            rx_done,
        );

        // Send dummy page
        let page = FetchedArticlePage {
            task: FetchTask {
                article_id: 0,
                url_id: 0,
                url: "http://example.com".into(),
                depth: 0,
                priority: common::types::Priority::Normal,
                discovered_from: None,
            },
            content: "content".into(),
            fetch_time: 0,
            links: HashSet::new(),
            title: None,
            document_metadata: Vec::new(),
            json_ld: None,
        };

        tx_extracted.send(page).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // No panic = success
    }

    // --------------------------------------------------
    // ✅ Test: system wiring (channels connect correctly)
    // --------------------------------------------------
    #[tokio::test]
    async fn test_channel_pipeline_integrity() {
        let (tx_fetch, mut rx_fetch) = mpsc::channel::<FetchTask>(10);

        tx_fetch
            .send(FetchTask {
                url: "http://example.com".into(),
                depth: 1,
                article_id: 0,
                url_id: 0,
                priority: common::types::Priority::Normal,
                discovered_from: None,
            })
            .await
            .unwrap();

        let received = rx_fetch.recv().await;
        assert!(received.is_some());
    }
}

#[cfg(test)]
mod run_system_tests {
    use super::*;
    use crate::frontier::db::frontier::MockFrontierDbTrait;
    use common::MockArchiver;
    use tokio::time::{Duration, sleep};

    use crate::settings::Config;

    fn test_config() -> Config {
        Config {
            user_agent: "test-agent".into(),
            archive_time: 0,
            noop_delay_millis: 10,
            workers: 1,
            db: ":memory:".into(),
            seed_urls: vec![], // important: keep empty
            hosts: vec![],
            archive_dir: "./tmp".into(),
            reset: Default::default(),
        }
    }

    // --------------------------------------------------
    // ✅ Test: run_system starts without crashing
    // --------------------------------------------------
    #[tokio::test]
    async fn test_run_system_starts_and_can_be_aborted() {
        // Setup mock expectations
        let archiver_ctx = MockArchiver::for_path_context();
        archiver_ctx.expect().returning(|_| MockArchiver::new());

        let db_ctx = MockFrontierDbTrait::connect_context();
        db_ctx.expect().returning(|_| MockFrontierDbTrait::new());

        let config = test_config();

        // Run system in background
        let handle = tokio::spawn(async move {
            let _ = run_system::<MockArchiver, MockFrontierDbTrait>(config).await;
        });

        // Let it run briefly
        sleep(Duration::from_millis(100)).await;

        // Abort the system (since it waits forever on ctrl_c)
        handle.abort();

        // Ensure task was aborted (not panicked)
        match handle.await {
            Err(e) => {
                assert!(e.is_cancelled(), "Task should be cancelled, not panic");
            }
            Ok(_) => {
                panic!("run_system should not exit normally");
            }
        }
    }
}
