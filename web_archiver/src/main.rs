use common::DefaultArchiver;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

mod config;
mod extractor;
mod fetcher;
mod frontier;
mod storage;

use common::types::{DiscoveredLinks, ExtractedPage, FetchTask, FetchedPage};
use extractor::parser::extractor_loop;
use fetcher::worker::worker_loop_single;
use frontier::db::frontier::FrontierDb;
use frontier::frontier_manager::FrontierManager;
use storage::archive::storage_loop;
use tokio::sync::Semaphore;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

use config::settings::Config;

#[tokio::main]
async fn main() {
    // --- 1. Initialize logging ---
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_thread_ids(true) // show thread IDs
        .with_thread_names(true) // show thread names
        .with_span_events(FmtSpan::NONE)
        .init();
    info!("Starting Web Archiver (Week 2 Skeleton)");

    // Load allowed domains config
    let config = Config::file("config.yaml").expect("Failed to load config.yaml");

    debug!(?config, "config");

    let noop_delay_millis = config.noop_delay_millis;
    let max_concurrent = config.workers;

    let conn = Connection::open("crawler.db").expect("failed to open DB");
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
    let (tx_extracted, rx_extracted) = mpsc::channel::<ExtractedPage>(100);
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
        let tx_fetched_clone = tx_fetched.clone();
        let sem = Arc::clone(&sem);
        async move {
            while let Some(task) = rx_fetch.recv().await {
                let tx_fetched_task = tx_fetched_clone.clone();
                let permit = sem.clone().acquire_owned().await.unwrap();
                let user_agent = config.user_agent.clone();
                tokio::spawn(async move {
                    worker_loop_single(task, &user_agent, tx_fetched_task).await;
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
    let archiver = DefaultArchiver::new();
    let storage_db = FrontierDb::new(db_arc.clone());
    tokio::spawn(async move {
        storage_loop(archiver, rx_extracted, storage_db).await;
    });

    // --- 8. Wait forever ---
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for Ctrl-C");
    info!("Shutting down");
}
