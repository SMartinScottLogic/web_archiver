use anyhow::Result;
use common::settings::CONFIG_FILE;
use settings::Config;
use tracing::{info, level_filters::LevelFilter, warn};
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};

mod aggregator;
mod archive_reader;
mod historical_serializer;
mod multi_page_merger;
mod settings;
mod url_utils;

use aggregator::ArchiveAggregator;
use archive_reader::ArchiveReader;
use historical_serializer::HistoricalSerializer;
use multi_page_merger::merge_pages_by_date;
use std::collections::HashMap;

fn setup_logging() {
    // Initialize logging
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_span_events(FmtSpan::NONE)
        .init();
}

fn main() -> Result<()> {
    setup_logging();

    let config =
        Config::file(CONFIG_FILE).unwrap_or_else(|_| panic!("Failed to load {}", CONFIG_FILE));

    info!("config: {:?}", config);
    info!("archive_dir: {}", config.archive_dir);
    info!("target_dir: {}", config.target_dir);

    let reader = ArchiveReader::new(&config.archive_dir, &config.target_dir);

    // Phase 1: Lightweight metadata scan (memory-efficient)
    info!("Scanning archive metadata (phase 1)...");
    let pages_by_domain = reader.read_page_paths_by_domain()?;
    let total_domains = pages_by_domain.len();
    let total_files: usize = pages_by_domain.values().map(|v| v.len()).sum();

    info!(
        "Archive metadata scanned: {} domains, {} total files",
        total_domains, total_files
    );

    // Phase 2-5: Process each domain separately to minimize peak memory usage
    let serializer = HistoricalSerializer::new(&config.target_dir);
    let mut global_files_read = 0;
    let mut global_files_failed = 0;
    let mut global_unique_urls = 0;
    let mut global_multi_page_urls = 0;
    let mut global_merged_snapshots = 0;
    let mut global_files_written = 0;

    for (domain_index, (domain, page_infos)) in pages_by_domain.iter().enumerate() {
        info!(
            "[{}/{}] Processing domain: {} ({} files)",
            domain_index + 1,
            total_domains,
            domain,
            page_infos.len()
        );

        // Phase 2: Load only this domain's pages into memory
        let mut aggregator = ArchiveAggregator::new();
        let mut files_read = 0;
        let mut files_failed = 0;

        for page_info in page_infos {
            match reader.load_page(&page_info.path) {
                Ok(page) => {
                    files_read += 1;
                    if !aggregator.add_page(page) {
                        warn!(url = %page_info.url, "failed to aggregate page (invalid URL)");
                    }
                }
                Err(error) => {
                    warn!(
                        path = ?page_info.path,
                        error = %error,
                        "failed to read page"
                    );
                    files_failed += 1;
                }
            }
        }

        let unique_urls = aggregator.unique_urls();
        let total_pages = aggregator.total_pages();

        info!(
            "Domain {} aggregation: {} files read, {} failed, {} unique URLs, {} total pages",
            domain, files_read, files_failed, unique_urls, total_pages
        );

        global_files_read += files_read;
        global_files_failed += files_failed;
        global_unique_urls += unique_urls;

        // Phase 3-4: Multi-page merging for this domain
        let aggregates = aggregator.into_aggregates();
        let mut domain_merged_snapshots_by_key: HashMap<
            aggregator::AggregateKey,
            Vec<multi_page_merger::MergedSnapshot>,
        > = HashMap::new();

        let mut domain_multi_page_urls = 0;
        for (key, page_entries) in &aggregates {
            let merged_by_date = merge_pages_by_date(page_entries);
            global_merged_snapshots += merged_by_date.len();

            if page_entries.len() > 1 {
                domain_multi_page_urls += 1;
            }

            let mut merged_list = Vec::new();
            for (_fetch_time, merged_snapshot) in merged_by_date {
                merged_list.push(merged_snapshot);
            }
            domain_merged_snapshots_by_key.insert(key.clone(), merged_list);
        }

        info!(
            "Domain {} merging complete: {} multi-page URLs, total merged snapshots so far: {}",
            domain, domain_multi_page_urls, global_merged_snapshots
        );
        global_multi_page_urls += domain_multi_page_urls;

        // Phase 5: Serialize this domain's pages
        let domain_files_written = serializer.serialize_all(&domain_merged_snapshots_by_key)?;
        global_files_written += domain_files_written;

        info!(
            "Domain {} serialization complete: {} files written",
            domain, domain_files_written
        );

        // Memory freed: aggregator and merged snapshots dropped at end of loop iteration
    }

    info!(
        "Archive processing complete: {} files read, {} failed, {} unique URLs, \
            {} multi-page URLs, {} total merged snapshots, {} output files written",
        global_files_read,
        global_files_failed,
        global_unique_urls,
        global_multi_page_urls,
        global_merged_snapshots,
        global_files_written
    );

    Ok(())
}
