use anyhow::Result;
use csv::{Writer, WriterBuilder};
use std::fs::{File, read_dir};
use std::path::Path;
use web_archiver::types::messages::ExtractedPage;

/// Generate a temporary CSV index of the archive
/// Each row: url, json_file_path
pub fn create_archive_index(archive_root: &str, output_csv: &str) -> Result<()> {
    let mut wtr = WriterBuilder::new()
        .delimiter(b'\t')
        .from_path(output_csv)?;

    // Write header
    wtr.write_record(["json_file_path", "url"])?;

    // Recursively scan archive folder
    scan_dir(Path::new(archive_root), &mut wtr)?;

    wtr.flush()?;
    Ok(())
}

/// Recursively scan a directory for JSON files and extract URLs
fn scan_dir(dir: &Path, wtr: &mut Writer<File>) -> Result<()> {
    for entry in read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, wtr)?;
        } else if path.extension().map(|ext| ext == "json").unwrap_or(false) {
            // Read JSON file to get the URL
            let file = File::open(&path)?;
            let page: ExtractedPage = serde_json::from_reader(file)?;
            wtr.write_record(&[path.to_string_lossy().to_string(), page.task.url.clone()])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Read;
    use tempfile::tempdir;
    use web_archiver::types::messages::{ExtractedPage, FetchTask, PageMetadata};

    #[test]
    fn test_create_archive_index_and_scan_dir() {
        let dir = tempdir().unwrap();
        let archive_root = dir.path().join("archive");
        fs::create_dir_all(&archive_root).unwrap();
        // Create a dummy JSON file
        let page = ExtractedPage {
            task: FetchTask {
                url_id: 1,
                url: "http://foo.com/test".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: Some("content".to_string()),
            links: vec![],
            metadata: PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 0,
                title: Some("Test".to_string()),
            },
        };
        let inner = "inner";
        fs::create_dir(archive_root.join(inner)).unwrap();
        let json_path = archive_root.join(inner).join("test.json");
        let file = File::create(&json_path).unwrap();
        serde_json::to_writer_pretty(file, &page).unwrap();

        let output_csv = dir.path().join("out.csv");
        let result =
            create_archive_index(archive_root.to_str().unwrap(), output_csv.to_str().unwrap());
        assert!(result.is_ok());

        // Check CSV output
        let mut csv_content = String::new();
        File::open(&output_csv)
            .unwrap()
            .read_to_string(&mut csv_content)
            .unwrap();
        assert!(csv_content.contains("json_file_path"));
        assert!(csv_content.contains("foo.com/test"));
    }
}
