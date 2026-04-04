use std::path::PathBuf;

use anyhow::Result;
use archive_migrator::{ensure_complete_in_db, process_file};
use clap::Parser;
use common::DefaultArchiver;
use rusqlite::Connection;
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(name = "archive_migrator")]
#[command(about = "Migrate archive files to new URL-based structure")]
struct Args {
    /// Run without making any filesystem changes
    #[arg(short, long)]
    dry_run: bool,

    /// Update database (ensure url recorded, set complete)
    #[arg(short, long)]
    update_db: bool,

    /// Root archive directory
    #[arg(short, long, default_value = "archive")]
    root: PathBuf,

    /// Destination archive directory
    #[arg(short, long, default_value = "archive")]
    archive_dir: PathBuf,

    /// Maximum number of files to process
    #[arg(short, long)]
    limit: Option<usize>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("Dry run: {}", args.dry_run);
    println!("Root: {:?}", args.root);
    println!("Archive: {:?}", args.archive_dir);

    let archiver = DefaultArchiver::new(args.archive_dir);

    let mut conn = Connection::open("crawler.db").expect("failed to open DB");

    for (processed, entry) in WalkDir::new(&args.root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .enumerate()
    {
        // Check limit BEFORE processing
        if let Some(limit) = args.limit
            && processed >= limit
        {
            println!("Reached limit ({} files), stopping.", limit);
            break;
        }

        if let Some(page) = process_file(&archiver, entry.path(), args.dry_run)?
            && args.update_db
            && !args.dry_run
        {
            ensure_complete_in_db(&*page, &mut conn)?;
        }
    }

    Ok(())
}
