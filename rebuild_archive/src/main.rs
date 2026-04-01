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

    info!("Starting archive reading and aggregation...");

    let pages = reader.read_all_pages();
    let mut aggregator = ArchiveAggregator::new();

    let mut files_read = 0;
    let mut files_failed = 0;

    for (path, result) in &pages {
        match result {
            Ok(page) => {
                let url = page.task.url.clone();
                files_read += 1;

                if aggregator.add_page(page.clone()) {
                    info!(url = %url, "aggregated page");
                } else {
                    warn!(url = %url, "failed to aggregate page (invalid URL)");
                }
            }
            Err(error) => {
                warn!(?path, error = %error, "failed to read page");
                files_failed += 1;
            }
        }
    }

    let unique_urls = aggregator.unique_urls();
    let total_pages = aggregator.total_pages();

    info!(
        "Archive aggregation complete: {} files read, {} failed, {} unique URLs, {} total pages",
        files_read, files_failed, unique_urls, total_pages
    );

    if total_pages > unique_urls {
        let multi_page_count = total_pages - unique_urls;
        info!(
            "Multi-page consolidation: {} pages will be merged into {} unique URLs",
            multi_page_count, unique_urls
        );
    }

    // Phase 2d: Multi-page merging
    info!("Starting multi-page merging...");

    let aggregates = aggregator.into_aggregates();
    let mut total_merged_snapshots = 0;
    let mut multi_page_urls = 0;
    let mut merged_snapshots_by_key: HashMap<
        aggregator::AggregateKey,
        Vec<multi_page_merger::MergedSnapshot>,
    > = HashMap::new();

    for (key, page_entries) in &aggregates {
        let merged_by_date = merge_pages_by_date(page_entries);
        total_merged_snapshots += merged_by_date.len();

        // Count URLs that had multiple pages merged
        if page_entries.len() > 1 {
            multi_page_urls += 1;
        }

        // Collect merged snapshots for serialization
        let mut merged_list = Vec::new();
        for (fetch_time, merged_snapshot) in merged_by_date {
            info!(
                domain = %key.domain,
                url = %key.normalized_url,
                year_month = format!("{}-{:02}", fetch_time.0, fetch_time.1),
                page_count = merged_snapshot.page_count,
                multi_page = ?page_entries.len() > 1,
                link_count = merged_snapshot.merged_links.len(),
                "merged pages"
            );
            merged_list.push(merged_snapshot);
        }
        merged_snapshots_by_key.insert(key.clone(), merged_list);
    }

    info!(
        "Multi-page merging complete: {} URLs with multiple pages, {} total merged snapshots",
        multi_page_urls, total_merged_snapshots
    );

    // Phase 2e: Serialize to HistoricalPage format
    info!("Starting historical page serialization...");

    let serializer = HistoricalSerializer::new(&config.target_dir);
    let files_written = serializer.serialize_all(&merged_snapshots_by_key)?;

    info!(files_written, "Historical page serialization complete",);

    Ok(())
}
