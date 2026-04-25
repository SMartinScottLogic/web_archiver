use std::path::PathBuf;

use tracing::debug;

use crate::{
    archiver::Archiver,
    page::PageReader,
    url::{remove_pagination_params, url_to_filename},
};

pub struct HistoricalArchiver {
    archive_dir: PathBuf,
}

impl Archiver for HistoricalArchiver {
    fn for_path(archive_dir: PathBuf) -> Self {
        Self { archive_dir }
    }

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
mod tests {
    use crate::page::MockPageReader;

    use super::*;
    use std::path::{Path, PathBuf};

    fn archiver() -> HistoricalArchiver {
        HistoricalArchiver::for_path(PathBuf::from("/archive"))
    }

    #[test]
    fn test_new_sets_archive_dir() {
        let dir = PathBuf::from("/tmp/test");
        let archiver = HistoricalArchiver::for_path(dir.clone());

        assert_eq!(archiver.archive_dir, dir);
    }

    #[test]
    fn test_generate_filename_basic() {
        let archiver = archiver();

        let mut page = MockPageReader::new();
        page.expect_url()
            .return_const("https://example.com/page".into());

        let path = archiver.generate_filename(&page).unwrap();
        let path_str = path.to_string_lossy();

        assert!(path_str.starts_with("/archive/"));
        assert!(path_str.ends_with(".json"));
    }

    #[test]
    fn test_canonical_filename_matches_generate_filename() {
        let archiver = archiver();
        let url = "https://example.com/page";

        let canonical = archiver.canonical_filename(url, 123).unwrap();

        let mut page = MockPageReader::new();
        page.expect_url().return_const(url.into());

        let generated = archiver.generate_filename(&page).unwrap();

        assert_eq!(canonical, generated);
    }

    #[test]
    fn test_store_page_calls_write_and_returns_path() {
        let archiver = archiver();

        let mut page = MockPageReader::new();

        page.expect_url()
            .return_const("https://example.com/page".into());

        page.expect_write()
            .withf(|path: &Path| path.extension().unwrap() == "json")
            .times(1)
            .returning(|_| Ok(()));

        let path = archiver.store_page(&page).unwrap();

        assert!(path.to_string_lossy().ends_with(".json"));
    }
    #[test]
    fn test_store_page_uses_generate_filename() {
        use std::sync::{Arc, Mutex};

        let archiver = archiver();

        let mut page = MockPageReader::new();
        page.expect_url()
            .return_const("https://example.com/page".into());

        // Capture the path passed to write()
        let written_path: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
        let wp = written_path.clone();

        page.expect_write().times(1).returning(move |path| {
            *wp.lock().unwrap() = Some(path.to_path_buf());
            Ok(())
        });

        let expected = archiver.generate_filename(&page).unwrap();
        let actual = archiver.store_page(&page).unwrap();

        let captured = written_path.lock().unwrap().clone();

        // --- Assertions ---
        assert_eq!(
            actual, expected,
            "returned path should match generated path"
        );

        assert!(
            captured.is_some(),
            "write() should have been called with a path"
        );

        assert_eq!(
            captured.unwrap(),
            expected,
            "write() should be called with the generated filename"
        );
    }

    #[test]
    fn test_pagination_params_removed() {
        let archiver = archiver();

        let url1 = "https://example.com/page?page=1";
        let url2 = "https://example.com/page?page=2";

        let path1 = archiver.canonical_filename(url1, 0).unwrap();
        let path2 = archiver.canonical_filename(url2, 0).unwrap();

        assert_eq!(path1, path2);
    }

    #[test]
    fn test_different_urls_produce_different_paths() {
        let archiver = archiver();

        let path1 = archiver
            .canonical_filename("https://example.com/a", 0)
            .unwrap();

        let path2 = archiver
            .canonical_filename("https://example.com/b", 0)
            .unwrap();

        assert_ne!(path1, path2);
    }

    #[test]
    fn test_filename_is_stable_across_timestamps() {
        let archiver = archiver();
        let url = "https://example.com/page";

        let p1 = archiver.canonical_filename(url, 1).unwrap();
        let p2 = archiver.canonical_filename(url, 999999).unwrap();

        assert_eq!(p1, p2);
    }

    #[test]
    fn test_write_error_propagates() {
        let archiver = archiver();

        let mut page = MockPageReader::new();

        page.expect_url()
            .return_const("https://example.com/page".into());

        page.expect_write()
            .times(1)
            .returning(|_| Err(anyhow::anyhow!("write failed")));

        let result = archiver.store_page(&page);

        assert!(result.is_err());
        let msg = format!("{:?}", result.err().unwrap());
        assert!(msg.contains("write failed"));
    }

    #[test]
    fn test_archive_dir_is_respected() {
        let archiver = HistoricalArchiver::for_path(PathBuf::from("/custom"));

        let mut page = MockPageReader::new();
        page.expect_url()
            .return_const("https://example.com/page".into());

        let path = archiver.generate_filename(&page).unwrap();

        assert!(path.starts_with("/custom"));
    }
}
