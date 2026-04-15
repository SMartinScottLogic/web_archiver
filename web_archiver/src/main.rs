use common::DefaultArchiver;
use common::types::{ArticleId, FetchTask};
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::level_filters::LevelFilter;

mod extractor;
mod fetcher;
mod frontier;
mod settings;

use extractor::parser::extractor_loop;
use fetcher::worker::worker_loop_single;
use frontier::db::frontier::FrontierDb;
use frontier::frontier_manager::FrontierManager;
use tokio::sync::Semaphore;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

use common::settings::CONFIG_FILE;
use settings::Config;

use crate::extractor::router::{FetchedArticlePage, Router};
use crate::extractor::{DiscoveredLinks, FetchedPage};

/// Initialize logging ---
fn setup_logging() {
    // Initialize logging
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy()
        .add_directive("html5ever::serialize=error".parse().unwrap());
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_span_events(FmtSpan::NONE)
        .init();
}

#[tokio::main]
async fn main() {
    setup_logging();
    info!("Starting Web Archiver (Migrated)");

    // Load allowed web_archiver config
    let config =
        Config::file(CONFIG_FILE).unwrap_or_else(|_| panic!("Failed to load {}", CONFIG_FILE));

    debug!(?config, "config");

    let noop_delay_millis = config.noop_delay_millis;
    let max_concurrent = config.workers;

    let conn = Connection::open(&config.db).expect("failed to open DB");
    frontier::db::schema::settings(&conn).expect("failed to set DB performance settings");
    frontier::db::schema::init_schema(&conn).expect("failed to init schema");
    let db_arc = Arc::new(Mutex::new(conn));

    // --- 2. Seed URLs ---
    let seed_urls = config.seed_urls.clone();

    // --- 3. Create channels ---
    // Frontier → Worker
    let (tx_fetch, mut rx_fetch) = mpsc::channel::<FetchTask>(100);
    // Worker → Extractor
    let (tx_fetched, rx_fetched) = mpsc::channel::<FetchedPage>(100);
    // Extractor → Storage
    let (tx_extracted, mut rx_extracted) = mpsc::channel::<FetchedArticlePage>(100);
    // Storage → Frontier
    let (tx_links, rx_links) = mpsc::channel::<DiscoveredLinks>(500);

    // --- 4. Spawn Frontier Manager ---
    let frontier_manager = FrontierManager::new(
        config.user_agent.clone(),
        seed_urls,
        tx_fetch,
        rx_links,
        noop_delay_millis,
        config.hosts,
        db_arc.clone(),
    );
    tokio::spawn(async move {
        frontier_manager.run().await;
    });

    // --- 5. Spawn Worker Tasks ---
    // This task owns the receiver and spawns multiple async fetch tasks
    let sem = Arc::new(Semaphore::new(max_concurrent));

    tokio::spawn({
        let archive_time = config.archive_time;
        let tx_fetched_clone = tx_fetched.clone();
        let sem = Arc::clone(&sem);
        async move {
            while let Some(task) = rx_fetch.recv().await {
                let tx_fetched_task = tx_fetched_clone.clone();
                let permit = sem.clone().acquire_owned().await.unwrap();
                let user_agent = config.user_agent.clone();
                tokio::spawn(async move {
                    worker_loop_single(task, archive_time, &user_agent, tx_fetched_task).await;
                    drop(permit); // release semaphore
                });
            }
        }
    });

    // --- 6. Spawn Extractor Task ---
    let tx_links_clone = tx_links.clone();
    tokio::spawn(async move {
        extractor_loop(rx_fetched, tx_extracted, tx_links_clone).await;
    });

    // --- 7. Spawn Storage Task ---
    let archiver = DefaultArchiver::new(PathBuf::from(config.archive_dir));
    let storage_db = FrontierDb::new(db_arc.clone());

    let (tx_done, mut rx_done) = mpsc::channel::<ArticleId>(100);

    let mut router = Router::new(archiver, storage_db, tx_done, max_concurrent * 2);

    // Router event loop
    tokio::spawn(async move {
        while let Some(page) = rx_extracted.recv().await {
            debug!(recieved = ?page, "route loop");
            router.route(page).await;

            // optional: clean up finished actors
            while let Ok(article_id) = rx_done.try_recv() {
                router.remove(article_id);
            }
        }
    });

    // --- 8. Wait forever ---
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for Ctrl-C");
    info!("Shutting down");
}
