use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use common::Archiver;
use common::DefaultArchiver;
use common::types::ExtractedPage;
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(name = "archive_migrator")]
#[command(about = "Migrate archive files to new URL-based structure")]
struct Args {
    /// Run without making any filesystem changes
    #[arg(long)]
    dry_run: bool,
    /// Root archive directory
    #[arg(long, default_value = "archive")]
    root: PathBuf,

    /// Maximum number of files to process
    #[arg(long)]
    limit: Option<usize>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("Dry run: {}", args.dry_run);
    println!("Root: {:?}", args.root);

    let archiver = DefaultArchiver::new();

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

        process_file(&archiver, entry.path(), args.dry_run)?;
    }

    Ok(())
}

fn process_file(archiver: &impl Archiver, path: &Path, dry_run: bool) -> Result<()> {
    // Read JSON
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(()),
    };

    let page: ExtractedPage = match serde_json::from_reader(file) {
        Ok(p) => p,
        Err(_) => return Ok(()),
    };

    let final_path = archiver.generate_filename(&page)?;

    if path == final_path {
        return Ok(());
    }

    println!("move from {:?} to {:?}", path, final_path);

    if dry_run {
        return Ok(());
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

    Ok(())
}
