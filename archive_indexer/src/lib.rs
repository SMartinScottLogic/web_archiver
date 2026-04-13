use anyhow::Result;
use csv::{Writer, WriterBuilder};
use indicatif::ProgressBar;
use serde_json::Value;
use std::fs::{File, read_dir};
use std::path::Path;

/// Generate a CSV index of the archive, supporting both ExtractedPage and HistoricalPage formats
/// Each row: json_file_path, url
///
/// This function is format-agnostic and can read either old ExtractedPage format
/// or new HistoricalPage format from the archive.
pub fn create_archive_index(archive_root: &str, output_csv: &str, pb: &ProgressBar) -> Result<()> {
    let mut wtr = WriterBuilder::new()
        .delimiter(b'\t')
        .from_path(output_csv)?;

    // Write header
    wtr.write_record(["json_file_path", "url"])?;

    // Recursively scan archive folder
    scan_dir(Path::new(archive_root), &mut wtr, pb)?;

    wtr.flush()?;
    Ok(())
}

/// Extract URL from either ExtractedPage or HistoricalPage JSON format
/// Both formats have the URL at different locations:
/// - ExtractedPage: task.url
/// - HistoricalPage: url (at root level)
fn extract_url_from_json(value: &Value) -> Option<String> {
    // Try ExtractedPage format first (nested in task.url)
    if let Some(url) = value
        .get("task")
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
    {
        return Some(url.to_string());
    }

    // Try HistoricalPage format (url at root level)
    if let Some(url) = value.get("url").and_then(|u| u.as_str()) {
        return Some(url.to_string());
    }

    None
}

/// Recursively scan a directory for JSON files and extract URLs
fn scan_dir(dir: &Path, wtr: &mut Writer<File>, pb: &ProgressBar) -> Result<()> {
    for entry in read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, wtr, pb)?;
        } else if path.extension().map(|ext| ext == "json").unwrap_or(false) {
            // Increment progress display
            pb.inc(1);

            // Read JSON file to extract URL (format-agnostic)
            let file = File::open(&path)?;
            match serde_json::from_reader::<_, Value>(file) {
                Ok(value) => {
                    if let Some(url) = extract_url_from_json(&value) {
                        wtr.write_record(&[path.to_string_lossy().to_string(), url])?;
                    }
                }
                Err(_) => {
                    // Skip files that can't be parsed as JSON
                    continue;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::historical::{HistoricalPage, HistoricalSnapshot};
    use common::types::{ExtractedPage, FetchTask, PageMetadata};
    use std::fs::{self, File};
    use std::io::Read;
    use tempfile::tempdir;

    fn create_test_extracted_page(url: &str) -> ExtractedPage {
        ExtractedPage {
            task: FetchTask {
                article_id: 1,
                url_id: 1,
                url: url.to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: Some("content".to_string()),
            links: vec![
                "https://example.com/link1".to_string(),
                "https://example.com/link2".to_string(),
            ],
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                title: Some("Test".to_string()),
                document_metadata: Some(vec![]),
            }),
        }
    }

    fn create_test_historical_page(url: &str) -> HistoricalPage {
        let base_page = create_test_extracted_page(url);
        let snapshot = HistoricalSnapshot::from_extracted_page(base_page);

        let url = url.to_string();
        let mut page = HistoricalPage::new(FetchTask { url, url_id: 0, article_id: 0, depth: 0, priority: 0, discovered_from: None });
        page.add_snapshot(snapshot);
        page
    }

    #[test]
    fn test_create_archive_index_with_extracted_pages() {
        let dir = tempdir().unwrap();
        let archive_root = dir.path().join("archive");
        fs::create_dir_all(&archive_root).unwrap();

        let page = create_test_extracted_page("https://example.com/test");

        let inner = "inner";
        fs::create_dir(archive_root.join(inner)).unwrap();
        let json_path = archive_root.join(inner).join("test.json");
        let file = File::create(&json_path).unwrap();
        serde_json::to_writer_pretty(file, &page).unwrap();

        let pb = ProgressBar::hidden();
        let output_csv = dir.path().join("out.csv");
        let result = create_archive_index(
            archive_root.to_str().unwrap(),
            output_csv.to_str().unwrap(),
            &pb,
        );
        assert!(result.is_ok());

        // Check CSV output
        let mut csv_content = String::new();
        File::open(&output_csv)
            .unwrap()
            .read_to_string(&mut csv_content)
            .unwrap();
        assert!(csv_content.contains("json_file_path"));
        assert!(csv_content.contains("example.com/test"));
    }

    #[test]
    fn test_create_archive_index_with_historical_pages() {
        let dir = tempdir().unwrap();
        let archive_root = dir.path().join("archive");
        fs::create_dir_all(&archive_root).unwrap();

        let page = create_test_historical_page("https://example.com/historical-test");

        let inner = "inner";
        fs::create_dir(archive_root.join(inner)).unwrap();
        let json_path = archive_root.join(inner).join("test.json");
        let file = File::create(&json_path).unwrap();
        serde_json::to_writer_pretty(file, &page).unwrap();

        let pb = ProgressBar::hidden();
        let output_csv = dir.path().join("out.csv");
        let result = create_archive_index(
            archive_root.to_str().unwrap(),
            output_csv.to_str().unwrap(),
            &pb,
        );
        assert!(result.is_ok());

        // Check CSV output
        let mut csv_content = String::new();
        File::open(&output_csv)
            .unwrap()
            .read_to_string(&mut csv_content)
            .unwrap();
        assert!(csv_content.contains("json_file_path"));
        assert!(csv_content.contains("example.com/historical-test"));
    }

    #[test]
    fn test_extract_url_from_extracted_page_format() {
        let page = create_test_extracted_page("https://example.com/test1");
        let value = serde_json::to_value(&page).unwrap();
        let url = extract_url_from_json(&value);
        assert_eq!(url, Some("https://example.com/test1".to_string()));
    }

    #[test]
    fn test_extract_url_from_historical_page_format() {
        let page = create_test_historical_page("https://example.com/test2");
        let value = serde_json::to_value(&page).unwrap();
        let url = extract_url_from_json(&value);
        assert_eq!(url, Some("https://example.com/test2".to_string()));
    }

    #[test]
    fn test_extract_url_from_invalid_json() {
        let value = serde_json::json!({"no_url": "here"});
        let url = extract_url_from_json(&value);
        assert_eq!(url, None);
    }

    #[test]
    fn test_archive_index_mixed_formats() {
        let dir = tempdir().unwrap();
        let archive_root = dir.path().join("archive");
        fs::create_dir_all(&archive_root).unwrap();

        // Create directory with mixed formats
        let inner = "inner";
        fs::create_dir(archive_root.join(inner)).unwrap();

        // Add an ExtractedPage
        let extracted = create_test_extracted_page("https://example.com/extracted");
        let extracted_path = archive_root.join(inner).join("extracted.json");
        let file = File::create(&extracted_path).unwrap();
        serde_json::to_writer_pretty(file, &extracted).unwrap();

        // Add a HistoricalPage
        let historical = create_test_historical_page("https://example.com/historical");
        let historical_path = archive_root.join(inner).join("historical.json");
        let file = File::create(&historical_path).unwrap();
        serde_json::to_writer_pretty(file, &historical).unwrap();

        let pb = ProgressBar::hidden();
        let output_csv = dir.path().join("out.csv");
        let result = create_archive_index(
            archive_root.to_str().unwrap(),
            output_csv.to_str().unwrap(),
            &pb,
        );
        assert!(result.is_ok());

        // Check CSV output contains both URLs
        let mut csv_content = String::new();
        File::open(&output_csv)
            .unwrap()
            .read_to_string(&mut csv_content)
            .unwrap();
        assert!(csv_content.contains("example.com/extracted"));
        assert!(csv_content.contains("example.com/historical"));
    }
}
