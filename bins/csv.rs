use web_archiver::types::messages::ExtractedPage;
use anyhow::Result;
use csv::{Writer, WriterBuilder};
use std::fs::{File, read_dir};
use std::path::Path;

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
