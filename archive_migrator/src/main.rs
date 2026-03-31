use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use common::Archiver;
use common::DefaultArchiver;
use common::types::ExtractedPage;
use common::url::{canonicalize_url, extract_domain};
use rusqlite::{Connection, params};
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
            ensure_complete_in_db(&page, &mut conn)?;
        }
    }

    Ok(())
}

fn process_file(
    archiver: &impl Archiver,
    path: &Path,
    dry_run: bool,
) -> Result<Option<ExtractedPage>> {
    // Read JSON
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };

    let mut page: ExtractedPage = match serde_json::from_reader(file) {
        Ok(p) => p,
        Err(_) => return Ok(None),
    };

    match canonicalize_url(&page.task.url) {
        Some(final_url) => page.task.url = final_url,
        None => {
            return Err(anyhow::Error::msg(format!(
                "failed to canonicalise {}",
                page.task.url
            )));
        }
    };

    let final_path = archiver.generate_filename(&page)?;

    if path == final_path {
        println!("keep in place {:?}", path);
        return Ok(Some(page));
    }

    println!("move from {:?} to {:?}", path, final_path);

    if dry_run {
        return Ok(Some(page));
    }

    // --- COPY ---
    fs::copy(path, &final_path).with_context(|| format!("Failed to copy to {:?}", final_path))?;

    // --- VERIFY (size check) ---
    let src_size = fs::metadata(path)?.len();
    let dst_size = fs::metadata(&final_path)?.len();

    if src_size != dst_size {
        anyhow::bail!(
            "Size mismatch after copy: {:?} ({} bytes) → {:?} ({} bytes)",
            path,
            src_size,
            final_path,
            dst_size
        );
    }

    // --- DELETE ORIGINAL ---
    fs::remove_file(path).with_context(|| format!("Failed to delete original {:?}", path))?;

    Ok(Some(page))
}

pub fn ensure_complete_in_db(page: &ExtractedPage, conn: &mut Connection) -> Result<()> {
    let url = &page.task.url;
    let fetch_time = page
        .metadata
        .as_ref()
        .map(|metadata| metadata.fetch_time)
        .unwrap_or_default() as i64;
    let tx = conn.transaction()?;
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO urls (url, domain, discovered_at) VALUES (?1, ?2, ?3)",
        params![&url, extract_domain(url).unwrap_or_default(), fetch_time],
    )?;
    if inserted > 0 {
        println!("   - added to DB");
    }
    let url_id: i64 = tx.query_row(
        "SELECT id FROM urls WHERE url = ?1",
        params![&url],
        |row: &rusqlite::Row<'_>| row.get(0),
    )?;
    // tx.execute(
    //     "INSERT OR IGNORE INTO frontier (url_id, priority, depth, discovered_from, status) VALUES (?1, ?2, ?3, ?4, 'complete')",
    //     params![url_id, page.task.priority, page.task.depth, page.task.discovered_from],
    // )?;
    tx.execute(
                "INSERT INTO frontier (url_id, priority, depth, discovered_from, status) VALUES (?1, ?2, ?3, ?4, 'complete') ON CONFLICT(url_id) DO UPDATE SET status = 'complete'", 
                params![url_id, page.task.priority, page.task.depth, page.task.discovered_from],
            )?;
    tx.commit()?;
    Ok(())
}
