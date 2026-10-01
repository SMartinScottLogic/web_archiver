use std::path::PathBuf;

use anyhow::Context;
use archive_time_backfill::{backfill_archive_timestamps, summarize_skipped_archive_files};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(about = "Backfill database fetch timestamps from archived article JSON")]
struct Args {
    /// Root directory containing archived article JSON files
    #[arg(long)]
    archive_dir: PathBuf,

    /// Existing crawler SQLite database file
    #[arg(long)]
    db: PathBuf,

    /// Apply updates; without this flag, only report what would change
    #[arg(long)]
    apply: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let report = backfill_archive_timestamps(&args.archive_dir, &args.db, args.apply)
        .context("backfill archive fetch times")?;

    println!("Mode: {}", if args.apply { "apply" } else { "dry run" });
    println!("JSON files scanned: {}", report.files_scanned);
    println!("Unique archive articles: {}", report.archive_articles);
    println!("Archive JSON files skipped: {}", report.skipped_files.len());
    println!("Database articles matched: {}", report.matched_articles);
    println!(
        "Frontier rows to update: {}",
        report.frontier_rows_to_update
    );
    if args.apply {
        println!("Frontier rows updated: {}", report.frontier_rows_updated);
    }
    println!(
        "Archive URLs without database matches: {}",
        report.unmatched_urls.len()
    );
    for url in report.unmatched_urls {
        eprintln!("No database article matches archive URL: {url}");
    }
    for summary in summarize_skipped_archive_files(&report.skipped_files) {
        eprintln!(
            "Skipped {} archive file(s): {} (example: {})",
            summary.count, summary.reason, summary.example_path
        );
    }
    Ok(())
}
