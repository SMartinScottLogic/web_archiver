use anyhow::Result;
use common::{DefaultArchiver, settings::CONFIG_FILE};
use itertools::Itertools;
use settings::Config;
use tracing::{debug, info, level_filters::LevelFilter, warn};
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};

mod aggregator;
mod archive_reader;
mod historical_serializer;
mod multi_page_merger;
mod settings;

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

    debug!("config: {:?}", config);
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

    // Archive statistics for understanding data distribution
    let mut domain_sizes: Vec<(String, usize)> = pages_by_domain
        .iter()
        .map(|(domain, infos)| (domain.clone(), infos.len()))
        .collect();
    domain_sizes.sort_by(|a, b| b.1.cmp(&a.1)); // Sort descending by file count

    let max_files = domain_sizes.first().map(|(_, count)| *count).unwrap_or(0);
    let min_files = domain_sizes.last().map(|(_, count)| *count).unwrap_or(0);
    let avg_files = if total_domains > 0 {
        total_files / total_domains
    } else {
        0
    };
    let concentration_pct = if total_files > 0 {
        (max_files as f64 / total_files as f64) * 100.0
    } else {
        0.0
    };

    info!("Archive distribution:");
    info!("  Domains: {}", total_domains);
    info!("  Total files: {}", total_files);
    info!("  Avg files per domain: {}", avg_files);
    info!(
        "  Largest domain: {} files ({:.1}% of total)",
        max_files, concentration_pct
    );
    info!("  Smallest domain: {} files", min_files);

    // Show top 5 domains
    info!("Top domains by size:");
    for (idx, (domain, count)) in domain_sizes.iter().take(5).enumerate() {
        let pct = (*count as f64 / total_files as f64) * 100.0;
        info!("  {}. {}: {} files ({:.1}%)", idx + 1, domain, count, pct);
    }

    // Warn if single domain is very dominant
    if concentration_pct > 80.0 {
        info!(
            "NOTE: {} contains {:.1}% of all files. Per-URL streaming is essential \
            to avoid memory exhaustion during processing. Domain-level batching would \
            attempt to load {} million pages simultaneously.",
            domain_sizes
                .first()
                .map(|(d, _)| d.clone())
                .unwrap_or_default(),
            concentration_pct,
            (max_files as f64 / 1_000_000.0).ceil() as i32
        );
    }

    // Phase 2-5: Process each domain separately, then by URL within domain
    let archiver = DefaultArchiver::new(config.target_dir.into());
    let serializer = HistoricalSerializer::new(archiver);
    let mut global_files_read = 0;
    let mut global_files_failed = 0;
    let mut global_unique_urls = 0;
    let mut global_multi_page_urls = 0;
    let mut global_merged_snapshots = 0;
    let mut global_files_written = 0;
    let mut global_files_deleted = 0;

    for (domain_index, (domain, page_infos)) in pages_by_domain.iter().enumerate() {
        info!(
            "[{}/{}] Processing domain: {} ({} files)",
            domain_index + 1,
            total_domains,
            domain,
            page_infos.len()
        );

        // Phase 2: Group pages by normalized URL (metadata only, no deserialization)
        let mut url_to_page_infos: HashMap<String, Vec<&archive_reader::PageInfo>> = HashMap::new();

        for page_info in page_infos {
            let normalized_url = match common::url::normalize_url_for_merge(&page_info.url) {
                Some(n) => n,
                None => {
                    warn!(url = %page_info.url, "failed to normalize URL");
                    global_files_failed += 1;
                    continue;
                }
            };
            url_to_page_infos
                .entry(normalized_url)
                .or_default()
                .push(page_info);
        }

        let urls_in_domain = url_to_page_infos.len();
        info!("Domain {} has {} unique URLs", domain, urls_in_domain);
        global_unique_urls += urls_in_domain;

        // Phase 3-4: For each URL in domain: load, aggregate, merge, serialize
        for (url_index, (normalized_url, url_page_infos)) in url_to_page_infos.iter().enumerate() {
            // Apply URL filter if configured
            if let Some(ref filter) = config.url_filter
                && !normalized_url.contains(filter)
            {
                info!(
                    "[{}/{}] Skipping URL {} (does not match filter '{}')",
                    url_index + 1,
                    urls_in_domain,
                    normalized_url,
                    filter
                );
                continue;
            }

            info!(
                "[{}/{}] Domain {} URL {}/{}: {} pages",
                url_index + 1,
                urls_in_domain,
                domain,
                url_index + 1,
                urls_in_domain,
                url_page_infos.len()
            );

            // Load only this URL's pages (deferred from metadata scan)
            let mut aggregator = ArchiveAggregator::new();
            let mut pages_read = 0;
            let mut pages_failed = 0;

            for page_info in url_page_infos {
                match reader.load_page(&page_info.path) {
                    // TODO Re-logic to also support HistoricalPage files
                    Ok(page) => {
                        pages_read += 1;
                        if !aggregator.add_page(page) {
                            warn!(url = %page_info.url, "failed to aggregate page");
                        }
                    }
                    Err(error) => {
                        warn!(
                            path = ?page_info.path,
                            error = %error,
                            "failed to read page"
                        );
                        pages_failed += 1;
                    }
                }
            }

            global_files_read += pages_read;
            global_files_failed += pages_failed;

            // Merge pages for this URL
            let aggregates = aggregator.into_aggregates();
            let mut url_merged_snapshots_by_key: HashMap<
                aggregator::AggregateKey,
                Vec<multi_page_merger::MergedSnapshot>,
            > = HashMap::new();

            for (key, page_entries) in aggregates {
                let merged_by_date = merge_pages_by_date(&page_entries);
                global_merged_snapshots += merged_by_date.len();

                if page_entries.len() > 1 {
                    global_multi_page_urls += 1;
                }

                let mut merged_list = Vec::new();
                for (_fetch_time, merged_snapshot) in merged_by_date
                    .into_iter()
                    .sorted_by_cached_key(|((year, date), _v)| year * 100 + date)
                {
                    merged_list.push(merged_snapshot);
                }
                url_merged_snapshots_by_key.insert(key, merged_list);
            }

            // Serialize this URL and free memory immediately
            let url_files_written = if config.update {
                serializer.serialize_all(&url_merged_snapshots_by_key)?
            } else {
                url_merged_snapshots_by_key.len()
            };
            global_files_written += url_files_written;

            // Cleanup source files after successful serialization if requested
            if config.cleanup && config.update {
                for page_info in url_page_infos {
                    match std::fs::remove_file(&page_info.path) {
                        Ok(_) => {
                            info!(
                                path = ?page_info.path,
                                "cleaned up source file"
                            );
                            global_files_deleted += 1;
                        }
                        Err(error) => {
                            warn!(
                                path = ?page_info.path,
                                error = %error,
                                "failed to remove source file during cleanup"
                            );
                        }
                    }
                }
            }

            // Memory freed: aggregator, merged_snapshots, url_page_infos dropped
        }

        info!(
            "Domain {} complete: {} unique URLs, {} merged snapshots so far",
            domain, urls_in_domain, global_merged_snapshots
        );
    }

    info!(
        "Archive processing complete: {} files read, {} failed, {} unique URLs, \
            {} multi-page URLs, {} total merged snapshots, {} output files written, \
            {} source files deleted",
        global_files_read,
        global_files_failed,
        global_unique_urls,
        global_multi_page_urls,
        global_merged_snapshots,
        global_files_written,
        global_files_deleted
    );

    Ok(())
}
