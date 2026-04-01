use anyhow::Result;
use common::settings::CONFIG_FILE;
use settings::Config;
use tracing::{info, warn, level_filters::LevelFilter};
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};

mod archive_reader;
mod settings;

use archive_reader::{ArchiveReader, ArchiveReaderStats};

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

    info!("Starting archive reading...");
    
    let pages = reader.read_all_pages();
    let mut stats = ArchiveReaderStats::default();
    
    for (path, result) in &pages {
        match result {
            Ok(page) => {
                info!(url = %page.task.url, "read page");
                stats.files_read += 1;
            }
            Err(error) => {
                warn!(?path, error = %error, "failed to read page");
                stats.files_failed += 1;
            }
        }
    }

    info!(
        "Archive reading complete: {} successful, {} failed",
        stats.files_read,
        stats.files_failed
    );

    Ok(())
}
