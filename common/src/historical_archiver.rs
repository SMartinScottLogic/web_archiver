use std::path::PathBuf;

use tracing::debug;

use crate::{
    Archiver,
    page::PageReader,
    url::{remove_pagination_params, url_to_filename},
};

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
        debug!(?path, "store_page");
        page.write(&path)?;
        Ok(path)
    }

    fn canonical_filename(&self, url_str: &str, _datetime: i64) -> anyhow::Result<PathBuf> {
        let url = url_str;
        let url = remove_pagination_params(url);
        let url = url_to_filename(&url);
        let path = self.archive_dir.join(format!("{}.json", url));
        Ok(path)
    }

    fn generate_filename(&self, page: &dyn PageReader) -> anyhow::Result<PathBuf> {
        let url = page.url();
        let url = remove_pagination_params(url);
        let url = url_to_filename(&url);
        let path = self.archive_dir.join(format!("{}.json", url));
        Ok(path)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use tracing_test::traced_test;

    #[test]
    #[traced_test]
    fn canonical_filename_for_path() {
        let archiver = HistoricalArchiver::new("/base".into());
        assert_eq!(
            archiver
                .canonical_filename("https://example.com/test/", 0)
                .unwrap()
                .to_string_lossy(),
            "/base/example.com/test/index.json"
        );
    }

    #[test]
    #[traced_test]
    fn canonical_filename_for_file() {
        let archiver = HistoricalArchiver::new("/base".into());
        assert_eq!(
            archiver
                .canonical_filename("https://example.com/test/file", 0)
                .unwrap()
                .to_string_lossy(),
            "/base/example.com/test/file.json"
        );
    }
}
