use std::collections::HashMap;
use std::path::PathBuf;
use std::{fs::File, time::Duration};

use common::types::ExtractedPage;
use indicatif::{ProgressBar, ProgressStyle};
use walkdir::WalkDir;

/// Lightweight page metadata without content (for memory-efficient scanning)
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct PageInfo {
    pub path: PathBuf,
    pub domain: String,
    pub url: String,
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

    /// Scan archive for page metadata WITHOUT loading content.
    /// Returns HashMap indexed by domain for memory-efficient batch processing.
    /// This is O(n) disk I/O but O(1) memory per file since content is not deserialized.
    pub fn read_page_paths_by_domain(&self) -> anyhow::Result<HashMap<String, Vec<PageInfo>>> {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template("{spinner} {pos} items [{elapsed}] ({per_sec}) {msg}")
                .unwrap(),
        );
        pb.enable_steady_tick(Duration::from_millis(100));
        pb.set_message("Scanning archive metadata...");

        let mut pages_by_domain: HashMap<String, Vec<PageInfo>> = HashMap::new();

        WalkDir::new(&self.archive_dir)
            .same_file_system(true)
            .sort_by_file_name()
            .into_iter()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if !entry.file_type().is_file() {
                    return None;
                }

                let path = entry.path().to_path_buf();

                // Quick parse: use Value to avoid full deserialization of content
                let result = match File::open(&path) {
                    Ok(file) => match serde_json::from_reader::<_, serde_json::Value>(file) {
                        Ok(obj) => {
                            let domain = obj["task"]["url"]
                                .as_str()
                                .and_then(common::url::extract_domain)
                                .unwrap_or_else(|| "unknown".to_string());
                            let url = obj["task"]["url"]
                                .as_str()
                                .map(|s| s.to_string())
                                .unwrap_or_default();

                            Some((domain, url))
                        }
                        Err(_) => None,
                    },
                    Err(_) => None,
                };

                if let Some((domain, url)) = result {
                    pages_by_domain
                        .entry(domain.clone())
                        .or_default()
                        .push(PageInfo {
                            path: path.clone(),
                            domain,
                            url,
                        });
                    pb.inc(1);
                }

                Some(())
            })
            .for_each(drop);

        pb.finish_with_message(format!("Scanned {} domains", pages_by_domain.len()));

        Ok(pages_by_domain)
    }

    /// Load a single ExtractedPage from disk by path
    pub fn load_page(&self, path: &PathBuf) -> Result<ExtractedPage, String> {
        match File::open(path) {
            Ok(file) => match serde_json::from_reader::<_, ExtractedPage>(file) {
                Ok(page) => Ok(page),
                Err(e) => Err(format!("Failed to deserialize JSON: {}", e)),
            },
            Err(e) => Err(format!("Failed to open file: {}", e)),
        }
    }

    /// (Deprecated) Walk the archive directory and collect all ExtractedPage files.
    /// WARNING: Loads ALL pages with full content into memory. Use read_page_paths_by_domain()
    /// and load_page() instead for large archives to reduce peak memory usage.
    #[allow(dead_code)]
    pub fn read_all_pages(&self) -> Vec<(PathBuf, Result<ExtractedPage, String>)> {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template("{spinner} {pos} items [{elapsed}] ({per_sec}) {msg}")
                .unwrap(),
        );
        pb.enable_steady_tick(Duration::from_millis(100));
        pb.set_message("Processing...");

        WalkDir::new(&self.archive_dir)
            .same_file_system(true)
            .sort_by_file_name()
            .into_iter()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if !entry.file_type().is_file() {
                    return None;
                }

                let path = entry.path().to_path_buf();
                let result = match File::open(&path) {
                    Ok(file) => match serde_json::from_reader::<_, ExtractedPage>(file) {
                        Ok(page) => Ok(page),
                        Err(e) => Err(format!("Failed to deserialize JSON: {}", e)),
                    },
                    Err(e) => Err(format!("Failed to open file: {}", e)),
                };
                pb.inc(1);
                pb.set_message(format!("read {:?}", path));
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
    fn test_page_info_creation() {
        let info = PageInfo {
            path: PathBuf::from("/archive/example.com/hash.json"),
            domain: "example.com".to_string(),
            url: "https://example.com/page".to_string(),
        };
        assert_eq!(info.domain, "example.com");
        assert_eq!(info.url, "https://example.com/page");
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
