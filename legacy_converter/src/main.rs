use clap::Parser;
use common::DefaultArchiver;
use legacy_converter::{parse_unambiguous_date, store_file};
use serde::Serialize;
use walkdir::WalkDir;

use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

/// Command line arguments
#[derive(Parser, Debug, Serialize)]
#[command(rename_all = "kebab-case")]
#[serde(rename_all = "snake_case")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Delete source files
    #[arg(short, long)]
    delete_source: bool,

    /// Date to use as the legacy crawl time
    #[arg(short, long, value_parser = parse_unambiguous_date)]
    fetch_time: u64,

    /// Root of legacy archive
    root: String,
}

#[tokio::main]
async fn main() {
    // Initialize logging ---
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_thread_ids(false) // show thread IDs
        .with_thread_names(false) // show thread names
        .with_span_events(FmtSpan::NONE)
        .init();

    // Read command line args
    let args = Args::parse();
    info!(?args, "Starting Web Archive conversion");

    let archiver = DefaultArchiver::new();

    for entry in WalkDir::new(args.root)
        .same_file_system(true)
        .into_iter()
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();

        match store_file(&archiver, path, args.fetch_time, args.delete_source) {
            Err(e) => error!(error = ?e, path = ?path, "Failed to migrate file"),
            Ok(_) => info!(path = ?path, "Migrated file"),
        }
    }
    info!("Shutting down");
}
