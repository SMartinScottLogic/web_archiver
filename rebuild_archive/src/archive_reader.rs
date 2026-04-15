use std::collections::HashMap;
use std::path::PathBuf;
use std::{fs::File, time::Duration};

use indicatif::{ProgressBar, ProgressStyle};
use rebuild_archive::extracted_page::ExtractedPage;
use walkdir::WalkDir;

/// Lightweight page metadata without content (for memory-efficient scanning)
#[derive(Clone, Debug)]
pub struct PageInfo {
    pub path: PathBuf,
    //pub domain: String,
    pub url: String,
}

/// ArchiveReader walks the existing hash-sharded archive and reads ExtractedPage files.
/// Provides iteration over all pages with error handling and statistics tracking.
pub struct ArchiveReader {
    archive_dir: PathBuf,
}

impl ArchiveReader {
    /// Create a new ArchiveReader for the given archive and output directories
    pub fn new(archive_dir: impl Into<PathBuf>) -> Self {
        Self {
            archive_dir: archive_dir.into(),
        }
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
                let result = File::open(&path)
                    .ok()
                    .and_then(|file| serde_json::from_reader::<_, serde_json::Value>(file).ok())
                    .and_then(|obj| {
                        obj["task"]["url"].as_str().and_then(|url| {
                            common::url::extract_domain(url).map(|domain| (domain, url.to_string()))
                        })
                    });

                if let Some((domain, url)) = result {
                    pages_by_domain
                        .entry(domain.clone())
                        .or_default()
                        .push(PageInfo {
                            path: path.clone(),
                            //domain,
                            url,
                        });
                    pb.inc(1);
                    pb.set_message(format!("{:?}", entry.path()));
                }

                Some(())
            })
            .for_each(drop);

        pb.finish_with_message(format!("Scanned {} domains", pages_by_domain.len()));

        Ok(pages_by_domain)
    }

    /// Load a single ExtractedPage from disk by path
    // TODO Re-logic to also support HistoricalPage files
    pub fn load_page(&self, path: &PathBuf) -> Result<ExtractedPage, String> {
        match File::open(path) {
            Ok(file) => match serde_json::from_reader::<_, ExtractedPage>(file) {
                Ok(page) => Ok(page),
                Err(e) => Err(format!("Failed to deserialize JSON: {}", e)),
            },
            Err(e) => Err(format!("Failed to open file: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_page_info_creation() {
        let info = PageInfo {
            path: PathBuf::from("/archive/example.com/hash.json"),
            //domain: "example.com".to_string(),
            url: "https://example.com/page".to_string(),
        };
        assert_eq!(info.url, "https://example.com/page");
    }

    /// Helper to create a JSON file with given content
    fn write_json_file(path: &std::path::Path, value: serde_json::Value) {
        let mut file = File::create(path).unwrap();
        write!(file, "{}", value).unwrap();
    }

    #[test]
    fn test_read_page_paths_by_domain_basic() {
        let dir = tempdir().unwrap();

        let file_path = dir.path().join("page1.json");

        let data = json!({
            "task": {
                "url": "https://example.com/page1"
            }
        });

        write_json_file(&file_path, data);

        let reader = ArchiveReader::new(dir.path());
        let result = reader.read_page_paths_by_domain().unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains_key("example.com"));

        let pages = &result["example.com"];
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].url, "https://example.com/page1");
    }

    #[test]
    fn test_read_page_paths_multiple_domains() {
        let dir = tempdir().unwrap();

        let file1 = dir.path().join("a.json");
        let file2 = dir.path().join("b.json");

        write_json_file(
            &file1,
            json!({
                "task": { "url": "https://example.com/a" }
            }),
        );

        write_json_file(
            &file2,
            json!({
                "task": { "url": "https://another.com/b" }
            }),
        );

        let reader = ArchiveReader::new(dir.path());
        let result = reader.read_page_paths_by_domain().unwrap();

        assert_eq!(result.len(), 2);
        assert!(result.contains_key("example.com"));
        assert!(result.contains_key("another.com"));
    }

    #[test]
    fn test_read_page_paths_invalid_json_skipped() {
        let dir = tempdir().unwrap();

        let valid = dir.path().join("valid.json");
        let invalid = dir.path().join("invalid.json");

        write_json_file(
            &valid,
            json!({
                "task": { "url": "https://example.com" }
            }),
        );

        // Write invalid JSON
        let mut file = File::create(&invalid).unwrap();
        write!(file, "{{ invalid json ").unwrap();

        let reader = ArchiveReader::new(dir.path());
        let result = reader.read_page_paths_by_domain().unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains_key("example.com"));
    }

    #[test]
    fn test_read_page_paths_missing_url() {
        let dir = tempdir().unwrap();

        let file_path = dir.path().join("missing.json");

        write_json_file(
            &file_path,
            json!({
                "task": {}
            }),
        );

        let reader = ArchiveReader::new(dir.path());
        let result = reader.read_page_paths_by_domain().unwrap();

        // Should skip unknown json
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_load_page_success_and_failure() {
        let dir = tempdir().unwrap();

        let valid_path = dir.path().join("valid.json");
        let invalid_path = dir.path().join("invalid.json");

        // NOTE: This assumes ExtractedPage can deserialize from this minimal structure.
        // If not, adjust to match your actual struct.
        write_json_file(&valid_path, valid_page_json("https://example.com"));

        let mut invalid_file = File::create(&invalid_path).unwrap();
        write!(invalid_file, "not json").unwrap();

        let reader = ArchiveReader::new(dir.path());

        let ok_result = reader.load_page(&valid_path);
        let err_result = reader.load_page(&invalid_path);

        // We don't assert Ok strictly since ExtractedPage schema may differ,
        // but we ensure failure path works
        assert!(ok_result.is_ok());
        assert!(err_result.is_err());
    }

    #[test]
    fn test_nested_directories_are_scanned() {
        let dir = tempdir().unwrap();
        let nested_dir = dir.path().join("nested");
        fs::create_dir(&nested_dir).unwrap();

        let file_path = nested_dir.join("page.json");

        write_json_file(
            &file_path,
            json!({
                "task": { "url": "https://nested.com/page" }
            }),
        );

        let reader = ArchiveReader::new(dir.path());
        let result = reader.read_page_paths_by_domain().unwrap();

        assert!(result.contains_key("nested.com"));
    }

    /// Create a fully valid ExtractedPage JSON
    fn valid_page_json(url: &str) -> serde_json::Value {
        json!({
            "task": {
                "url_id": 1,
                "url": url,
                "depth": 0,
                "priority": 0,
                "discovered_from": null
            },
            "content_markdown": "hello world",
            "links": ["https://example.com/other"],
            "metadata": {
                "status_code": 200,
                "content_type": "text/html",
                "fetch_time": 123456,
                "title": "Example",
                "document_metadata": null
            }
        })
    }

    fn write_json(path: &std::path::Path, value: serde_json::Value) {
        let mut file = File::create(path).unwrap();
        write!(file, "{}", value).unwrap();
    }

    #[test]
    fn test_load_page_success_full_validation() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("page.json");

        write_json(&file_path, valid_page_json("https://example.com"));

        let reader = ArchiveReader::new(dir.path());
        let page = reader.load_page(&file_path).unwrap();

        assert_eq!(page.task.url, "https://example.com");
        assert_eq!(page.task.url_id, 1);
        assert_eq!(page.links.len(), 1);
        assert_eq!(page.content_markdown.as_deref(), Some("hello world"));

        let metadata = page.metadata.unwrap();
        assert_eq!(metadata.status_code, 200);
        assert_eq!(metadata.title.as_deref(), Some("Example"));
    }

    #[test]
    fn test_load_page_file_not_found() {
        let reader = ArchiveReader::new("archive");
        let path = std::path::PathBuf::from("non_existent.json");

        let result = reader.load_page(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to open file"));
    }

    #[test]
    fn test_load_page_invalid_json_error_message() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("bad.json");

        let mut file = File::create(&file_path).unwrap();
        write!(file, "not valid json").unwrap();

        let reader = ArchiveReader::new(dir.path());
        let result = reader.load_page(&file_path);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to deserialize JSON"));
    }

    #[test]
    fn test_read_page_paths_preserves_paths() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("page.json");

        write_json(&file_path, valid_page_json("https://example.com"));

        let reader = ArchiveReader::new(dir.path());
        let result = reader.read_page_paths_by_domain().unwrap();

        let pages = &result["example.com"];
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].path, file_path);
    }

    #[test]
    fn test_read_page_paths_multiple_files_same_domain() {
        let dir = tempdir().unwrap();

        let f1 = dir.path().join("1.json");
        let f2 = dir.path().join("2.json");

        write_json(&f1, valid_page_json("https://example.com/a"));
        write_json(&f2, valid_page_json("https://example.com/b"));

        let reader = ArchiveReader::new(dir.path());
        let result = reader.read_page_paths_by_domain().unwrap();

        let pages = &result["example.com"];
        assert_eq!(pages.len(), 2);

        let urls: Vec<_> = pages.iter().map(|p| p.url.as_str()).collect();
        assert!(urls.contains(&"https://example.com/a"));
        assert!(urls.contains(&"https://example.com/b"));
    }

    #[test]
    fn test_read_page_paths_empty_directory() {
        let dir = tempdir().unwrap();

        let reader = ArchiveReader::new(dir.path());
        let result = reader.read_page_paths_by_domain().unwrap();

        assert!(result.is_empty());
    }
}
