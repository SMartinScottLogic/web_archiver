use std::fs::File;
use std::path::PathBuf;

use common::types::ExtractedPage;
use walkdir::WalkDir;

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
#[allow(dead_code)]
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
    #[allow(dead_code)]
    pub fn stats(&self) -> &ArchiveReaderStats {
        &self.stats
    }

    /// Get mutable statistics (for updates during processing)
    #[allow(dead_code)]
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
                        Ok(page) => Ok(page),
                        Err(e) => Err(format!("Failed to deserialize JSON: {}", e)),
                    },
                    Err(e) => Err(format!("Failed to open file: {}", e)),
                };

                Some((path, result))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_archive_reader_creation() {
        let reader = ArchiveReader::new("archive", "output");
        assert_eq!(reader.stats().files_read, 0);
        assert_eq!(reader.stats().files_failed, 0);
        assert_eq!(reader.stats().unique_urls, 0);
    }

    #[test]
    fn test_archive_reader_stats() {
        let mut reader = ArchiveReader::new("archive", "output");
        reader.stats_mut().files_read = 5;
        reader.stats_mut().files_failed = 2;

        assert_eq!(reader.stats().files_read, 5);
        assert_eq!(reader.stats().files_failed, 2);
    }
}
