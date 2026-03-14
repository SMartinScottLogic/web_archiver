use clap::Parser;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

mod config;
mod extractor;
mod fetcher;
mod frontier;
mod storage;
mod types;
mod util;

use extractor::parser::extractor_loop;
use fetcher::worker::worker_loop_single;
use frontier::db::frontier::FrontierDb;
use frontier::frontier_manager::FrontierManager;
use storage::archive::storage_loop;
use tokio::sync::Semaphore;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use types::messages::{DiscoveredLinks, ExtractedPage, FetchTask, FetchedPage};

use config::settings::DomainConfig;

/// Command line arguments
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Delay in ms for frontier manager idle loop
    #[arg(long, default_value_t = 500)]
    noop_delay_millis: u64,

    /// Number of concurrent fetch workers (overrides config if set)
    #[arg(long)]
    workers: Option<usize>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Load allowed domains config
    let domain_config =
        DomainConfig::load_from_file("config.yaml").expect("Failed to load allowed_domains.yaml");

    let noop_delay_millis = args.noop_delay_millis;

    // Use CLI value if present, else config, else fallback
    let max_concurrent = args.workers.or(domain_config.workers).unwrap_or(1);

    let conn = Connection::open("crawler.db").expect("failed to open DB");
    frontier::db::schema::init_schema(&conn).expect("failed to init schema");
    let db_arc = Arc::new(Mutex::new(conn));

    // --- 1. Initialize logging ---
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_thread_ids(true) // show thread IDs
        .with_thread_names(true) // show thread names
        .with_span_events(FmtSpan::NONE)
        .init();
    info!("Starting Web Archiver (Week 1 Skeleton)");

    // --- 2. Seed URLs ---
    let seed_urls = domain_config.seed_urls.clone().unwrap_or_default();

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
        seed_urls,
        tx_fetch.clone(),
        rx_links,
        noop_delay_millis,
        domain_config.hosts,
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
                tokio::spawn(async move {
                    worker_loop_single(task, tx_fetched_task).await;
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
    let storage_db = FrontierDb::new(db_arc.clone());
    tokio::spawn(async move {
        storage_loop(rx_extracted, storage_db).await;
    });

    // --- 8. Wait forever ---
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for Ctrl-C");
    info!("Shutting down");
}
