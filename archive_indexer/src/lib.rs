use anyhow::Result;
use csv::{Writer, WriterBuilder};
use indicatif::ProgressBar;
use serde_json::Value;
use serde_json_path::JsonPath;
use std::fs::{File, read_dir};
use std::path::Path;
use std::time::Duration;

use clap::Parser;
use indicatif::ProgressStyle;

#[derive(Parser, Debug)]
#[clap(
    name = "archive_indexer",
    version = "0.1.1",
    about = "Create an index of archive files"
)]
pub struct Args {
    /// Archive root directory
    #[clap(value_name = "ARCHIVE_ROOT")]
    pub archive_root: String,

    /// Output CSV file
    #[clap(value_name = "OUTPUT_CSV")]
    pub output_csv: String,
}

/// Main Application
pub fn run(args: Args) -> Result<()> {
    let pb = ProgressBar::new_spinner();

    pb.set_style(
        ProgressStyle::with_template("{spinner} {pos} items [{elapsed}] ({per_sec}) {msg}")
            .unwrap(),
    );

    pb.enable_steady_tick(Duration::from_millis(100));
    pb.set_message("Processing...");

    create_archive_index(&args.archive_root, &args.output_csv, &pb)?;

    pb.finish_with_message("Done");

    println!("Archive index written to {}", args.output_csv);
    Ok(())
}

/// Generate a CSV index of the archive, supporting all supported formats (WithTask-based)
/// Each row: json_file_path, url
///
/// This function is relatively format-agnostic.
pub fn create_archive_index(archive_root: &str, output_csv: &str, pb: &ProgressBar) -> Result<()> {
    let mut wtr = WriterBuilder::new()
        .delimiter(b'\t')
        .from_path(output_csv)?;

    // Write header
    wtr.write_record(["json_file_path", "url", "authors"])?;

    // Recursively scan archive folder
    scan_dir(Path::new(archive_root), &mut wtr, pb)?;

    wtr.flush()?;
    Ok(())
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
                    if let Ok((url, authors)) = get_data_points(&value) {
                        //println!("json path | url = {:?}, authors = {:?}", url, authors);
                        wtr.write_record([path.to_str().unwrap_or_default(), &url, &authors])?;
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

fn get_data_points(value: &Value) -> anyhow::Result<(String, String)> {
    let url_path = JsonPath::parse("$.task.url")?;
    let authors_path = JsonPath::parse("$.current.metadata.authors")?;
    let url = url_path
        .query(value)
        .first()
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let authors = authors_path
        .query(value)
        .first()
        .cloned()
        .and_then(|v| {
            v.as_array().map(|e| {
                e.iter()
                    .map(|e| e.as_str().map(|s| s.to_string()))
                    .collect::<Option<Vec<_>>>()
                    .unwrap_or_default()
            })
        })
        .unwrap_or_default()
        .join(",");
    Ok((url, authors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::historical::{HistoricalContent, HistoricalPage, HistoricalSnapshot};
    use common::types::{FetchTask, PageMetadata, Priority, WithTask};
    use std::collections::HashSet;
    use std::fs::{self, File};
    use std::io::Read;
    use tempfile::tempdir;

    fn create_test_extracted_page(url: &str) -> WithTask {
        WithTask {
            task: FetchTask {
                article_id: 1,
                url_id: 1,
                url: url.to_string(),
                depth: 0,
                priority: Priority::default(),
                discovered_from: None,
            },
        }
    }

    fn create_test_historical_page(url: &str) -> HistoricalPage {
        let snapshot = HistoricalSnapshot {
            content_markdown: vec![HistoricalContent {
                page: 1,
                content: common::historical::HistoricalContentType::Literal("content".to_string()),
            }],
            links: HashSet::from([
                "https://example.com/link1".to_string(),
                "https://example.com/link2".to_string(),
            ]),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 1000,
                authors: Vec::new(),
                title: Some("Test".to_string()),
                document_metadata: Some(vec![]),
                json_ld: None,
            }),
        };

        let url = url.to_string();
        let mut page = HistoricalPage::new(FetchTask {
            url,
            url_id: 0,
            article_id: 0,
            depth: 0,
            priority: Priority::default(),
            discovered_from: None,
        });
        page.add_snapshot(snapshot).unwrap();
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
    fn test_archive_index_mixed_formats() {
        let dir = tempdir().unwrap();
        let archive_root = dir.path().join("archive");
        fs::create_dir_all(&archive_root).unwrap();

        // Create directory with mixed formats
        let inner = "inner";
        fs::create_dir(archive_root.join(inner)).unwrap();

        // Add a test page
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
