use std::path::PathBuf;

use crate::{Archiver, page::PageReader, url::url_to_filename};

pub struct HistoricalArchiver {
    archive_dir: PathBuf,
}

impl HistoricalArchiver {
    pub fn new(archive_dir: PathBuf) -> Self {
        Self { archive_dir }
    }
}

impl Archiver for HistoricalArchiver {
    fn store_page(&self, page: &dyn PageReader) -> anyhow::Result<PathBuf> {
        let path = self.generate_filename(page)?;
        page.write(&path)?;
        Ok(path)
    }

    fn canonical_filename(&self, url_str: &str, _datetime: i64) -> anyhow::Result<PathBuf> {
        let url = url_str;
        let url_filename = url_to_filename(url);
        let path = self.archive_dir.join(format!("{}.json", url_filename));
        Ok(path)
    }

    fn generate_filename(&self, page: &dyn PageReader) -> anyhow::Result<PathBuf> {
        let url = page.url();
        let url_filename = url_to_filename(url);
        let path = self.archive_dir.join(format!("{}.json", url_filename));
        Ok(path)
    }
}
