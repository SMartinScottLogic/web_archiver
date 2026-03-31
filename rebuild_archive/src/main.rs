use std::fs::File;

use anyhow::Result;
//use common::{settings::{CONFIG_FILE, CommonArgs, Config},
use common::{settings::CONFIG_FILE, types::ExtractedPage};
use settings::Config;
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};
use walkdir::WalkDir;

mod settings;

fn setup_logging() {
    // Initialize logging ---
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_thread_ids(false) // show thread IDs
        .with_thread_names(false) // show thread names
        .with_span_events(FmtSpan::NONE)
        .init();
}

fn main() -> Result<()> {
    setup_logging();

    let config =
        Config::file(CONFIG_FILE).unwrap_or_else(|_| panic!("Failed to load {}", CONFIG_FILE));

    info!("config: {:?}", config);

    for entry in WalkDir::new(config.archive_dir)
        .same_file_system(true)
        .into_iter()
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        // Read JSON file to get the URL
        let file = File::open(path)?;
        let page: ExtractedPage = serde_json::from_reader(file)?;
        info!(page.task.url, "read");
    }
    Ok(())
}
