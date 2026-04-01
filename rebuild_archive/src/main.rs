use std::fs::File;
use std::path::PathBuf;

use anyhow::Result;
use common::{settings::CONFIG_FILE, types::ExtractedPage};
use settings::Config;
use tracing::{info, warn, level_filters::LevelFilter};
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};
use walkdir::WalkDir;

mod settings;

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

/// ArchiveReader walks the existing hash-sharded archive and reads ExtractedPage files.
/// Provides iteration over all pages with error handling and statistics tracking.
pub struct ArchiveReader {
    archive_dir: PathBuf,
    /// Output directory for rebuilt archive (used in Phase 2 onwards)
    #[allow(dead_code)]
    output_dir: PathBuf,
    /// Statistics tracking
    stats: ArchiveReaderStats,
}

#[derive(Debug, Clone, Default)]
pub struct ArchiveReaderStats {
    pub files_read: usize,
    pub files_failed: usize,
    pub unique_urls: usize,
}

impl ArchiveReader {
    /// Create a new ArchiveReader for the given archive and output directories
    pub fn new(archive_dir: impl Into<PathBuf>, output_dir: impl Into<PathBuf>) -> Self {
        Self {
            archive_dir: archive_dir.into(),
            output_dir: output_dir.into(),
            stats: ArchiveReaderStats::default(),
        }
    }

    /// Get current statistics
    pub fn stats(&self) -> &ArchiveReaderStats {
        &self.stats
    }

    /// Get mutable statistics (for updates during processing)
    pub fn stats_mut(&mut self) -> &mut ArchiveReaderStats {
        &mut self.stats
    }

    /// Walk the archive directory and collect all ExtractedPage files
    /// 
    /// Returns a Vec of (PathBuf, Result<ExtractedPage>) tuples.
    /// Errors are included in the Vec rather than stopping iteration.
    pub fn read_all_pages(&self) -> Vec<(PathBuf, Result<ExtractedPage, String>)> {
        WalkDir::new(&self.archive_dir)
            .same_file_system(true)
            .into_iter()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if !entry.file_type().is_file() {
                    return None;
                }
                
                let path = entry.path().to_path_buf();
                
                // Try to read and deserialize the file
                let result = match File::open(&path) {
                    Ok(file) => match serde_json::from_reader::<_, ExtractedPage>(file) {
                        Ok(page) => {
                            Ok(page)
                        }
                        Err(e) => {
                            Err(format!("Failed to deserialize JSON: {}", e))
                        }
                    },
                    Err(e) => {
                        Err(format!("Failed to open file: {}", e))
                    }
                };
                
                Some((path, result))
            })
            .collect()
    }
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
