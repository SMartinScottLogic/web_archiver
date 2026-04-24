use std::{
    fs::{File, create_dir_all},
    path::PathBuf,
};

use anyhow::Context as _;
use chrono::{DateTime, Datelike as _, Utc};
use url::Url;

use crate::{
    Archiver,
    page::PageReader,
    types::WithTask,
    url::{hash_url, sanitize_segment},
};

pub struct FullPathArchiver {
    archive_dir: PathBuf,
}

impl Archiver for FullPathArchiver {
    fn for_path(archive_dir: PathBuf) -> Self {
        Self { archive_dir }
    }

    fn store_page(&self, page: &dyn PageReader) -> anyhow::Result<PathBuf> {
        let path = self.generate_filename(page)?;
        page.write(&path)?;
        Ok(path)
    }

    fn canonical_filename(&self, url_str: &str, datetime: i64) -> anyhow::Result<PathBuf> {
        let url = Url::parse(url_str).with_context(|| format!("Invalid URL: {}", url_str))?;

        // --- Time ---
        let datetime = DateTime::<Utc>::from_timestamp(datetime, 0)
            .ok_or_else(|| anyhow::anyhow!("Invalid timestamp: {}", datetime))?;

        // --- Path segments ---
        let segments: Vec<_> = url
            .path_segments()
            .map(|s| s.collect::<Vec<_>>())
            .unwrap_or_default();

        // --- Base path ---
        let mut base_path = self.archive_dir.clone();

        let domain = url.domain().unwrap_or("unknown");
        base_path.push(domain);

        for seg in segments {
            let clean = sanitize_segment(seg);
            if !clean.is_empty() {
                base_path.push(clean);
            }
        }

        // --- Hash ---
        let hash: String = hash_url(url_str);

        // --- Sharding ---
        base_path.push(&hash[0..2]);
        base_path.push(&hash[2..4]);

        // --- Filename ---
        let filename = format!("{}_{}-{:02}.json", hash, datetime.year(), datetime.month());
        let path = base_path.join(filename);
        Ok(path)
    }

    fn generate_filename(&self, page: &dyn PageReader) -> anyhow::Result<PathBuf> {
        let current = page
            .current()
            .as_ref()
            .ok_or_else(|| anyhow::Error::msg("Failed to get current snapshot"))?;
        let url_str = page.url();

        let url = Url::parse(url_str).with_context(|| format!("Invalid URL: {}", url_str))?;

        // --- Time ---
        let datetime = current
            .metadata
            .clone()
            .map(|metadata| metadata.fetch_time)
            .unwrap_or_default();
        let datetime = DateTime::<Utc>::from_timestamp(datetime as i64, 0)
            .ok_or_else(|| anyhow::anyhow!("Invalid timestamp: {}", datetime))?;

        // --- Path segments ---
        let segments: Vec<_> = url
            .path_segments()
            .map(|s| s.collect::<Vec<_>>())
            .unwrap_or_default();

        // --- Base path ---
        let mut base_path = self.archive_dir.clone();

        let domain = url.domain().unwrap_or("unknown");
        base_path.push(domain);

        for seg in segments {
            let clean = sanitize_segment(seg);
            if !clean.is_empty() {
                base_path.push(clean);
            }
        }

        // --- Hash ---
        let hash: String = hash_url(url_str);

        // --- Sharding ---
        base_path.push(&hash[0..2]);
        base_path.push(&hash[2..4]);

        create_dir_all(&base_path)?;

        // --- Filename ---
        let mut attempt = 0;

        loop {
            let filename = if attempt == 0 {
                format!("{}_{}-{:02}.json", hash, datetime.year(), datetime.month())
            } else {
                format!(
                    "{}_{}-{:02}_{}.json",
                    hash,
                    datetime.year(),
                    datetime.month(),
                    attempt
                )
            };

            let path = base_path.join(filename);

            if !path.exists() {
                return Ok(path);
            }

            // Check for same URL (collision vs duplicate)
            if let Some(existing) = File::open(&path)
                .ok()
                .and_then(|f| serde_json::from_reader::<_, WithTask>(f).ok())
                && existing.task.url == *url_str
            {
                // overwrite same URL
                return Ok(path);
            }

            attempt += 1;

            if attempt > 100 {
                anyhow::bail!("Too many collisions for {}", hash);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::historical::HistoricalSnapshot;
    use crate::page::MockPageReader;
    use crate::types::{FetchTask, PageMetadata, Priority};
    use mockall::predicate::*;
    use std::collections::HashSet;
    use std::{fs, path::Path};

    fn make_snapshot(_url: &str, ts: u64) -> HistoricalSnapshot {
        HistoricalSnapshot {
            // task: FetchTask {
            //     url_id: 1,
            //     url: url.to_string(),
            //     depth: 0,
            //     priority: 0,
            //     discovered_from: None,
            // },
            content_markdown: Vec::new(),
            links: HashSet::new(),
            metadata: Some(PageMetadata {
                status_code: 200,
                content_type: Some("text/html".into()),
                fetch_time: ts,
                title: None,
                document_metadata: None,
            }),
        }
    }

    fn cleanup(path: &Path) {
        if path.exists() {
            let _ = fs::remove_dir_all(path);
        }
    }

    fn test_dir() -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push("full_path_archiver_tests");
        dir.push(format!("test-{}", uuid::Uuid::new_v4()));
        dir
    }

    #[test]
    fn test_generate_filename_basic() {
        let base = test_dir();
        let archiver = FullPathArchiver::for_path(base.clone());

        let url = "https://example.com/a/b";
        let snapshot = make_snapshot(url, 1700000000);

        let mut mock = MockPageReader::new();
        mock.expect_current().return_const(Some(snapshot));
        mock.expect_url().return_const(url.to_owned());

        let path = archiver.generate_filename(&mock).unwrap();
        let path_str = path.to_string_lossy();

        assert!(path_str.contains("archive"));
        assert!(path_str.contains("example.com"));
        assert!(path_str.contains("a"));
        assert!(path_str.contains("b"));

        cleanup(&base);
    }

    #[test]
    fn test_generate_filename_includes_date() {
        let base = test_dir();
        let archiver = FullPathArchiver::for_path(base.clone());

        let url = "https://example.com";
        let snapshot = make_snapshot(url, 1700000000);

        let mut mock = MockPageReader::new();
        mock.expect_current().return_const(Some(snapshot));
        mock.expect_url().return_const(url.to_owned());

        let path = archiver.generate_filename(&mock).unwrap();
        let filename = path.file_name().unwrap().to_string_lossy();

        assert!(filename.contains("2023-11"));

        cleanup(&base);
    }

    #[test]
    fn test_generate_filename_invalid_url() {
        let base = test_dir();
        let archiver = FullPathArchiver::for_path(base.clone());

        let url = "not a url";
        let snapshot = make_snapshot(url, 1700000000);

        let mut mock = MockPageReader::new();
        mock.expect_current().return_const(Some(snapshot));
        mock.expect_url().return_const(url.to_owned());

        let result = archiver.generate_filename(&mock);

        assert!(result.is_err());
        cleanup(&base);
    }

    #[test]
    fn test_generate_filename_invalid_timestamp() {
        let base = test_dir();
        let archiver = FullPathArchiver::for_path(base.clone());

        let url = "https://example.com";
        let snapshot = make_snapshot(url, u64::MAX);

        let mut mock = MockPageReader::new();
        mock.expect_current().return_const(Some(snapshot));
        mock.expect_url().return_const(url.to_owned());

        let result = archiver.generate_filename(&mock);

        assert!(result.is_ok());
        assert!(result.unwrap().to_str().unwrap().ends_with("_1969-12.json"));
        cleanup(&base);
    }

    #[test]
    fn test_generate_filename_no_current_snapshot() {
        let base = test_dir();
        let archiver = FullPathArchiver::for_path(base.clone());

        let mut mock = MockPageReader::new();
        mock.expect_current().return_const(None);

        let result = archiver.generate_filename(&mock);

        assert!(result.is_err());
        cleanup(&base);
    }

    #[test]
    fn test_store_page_calls_write() {
        let base = test_dir();
        let archiver = FullPathArchiver::for_path(base.clone());

        let url = "https://example.com/write";
        let snapshot = make_snapshot(url, 1700000000);

        let mut mock = MockPageReader::new();

        mock.expect_current().return_const(Some(snapshot));
        mock.expect_url().return_const(url.to_owned());

        mock.expect_write().times(1).returning(|path| {
            // simulate writing a valid file
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(path, b"{}")?;
            Ok(())
        });

        let path = archiver.store_page(&mock).unwrap();

        assert!(path.exists());

        cleanup(&base);
    }

    #[test]
    fn test_same_url_overwrites() {
        let base = test_dir();
        let archiver = FullPathArchiver::for_path(base.clone());

        let url = "https://example.com/same";
        let snapshot = make_snapshot(url, 1700000000);

        let mut mock1 = MockPageReader::new();
        mock1.expect_current().return_const(Some(snapshot.clone()));
        mock1.expect_url().return_const(url.to_owned());
        mock1.expect_write().returning(|path| {
            fs::create_dir_all(path.parent().unwrap())?;
            let page = WithTask {
                task: FetchTask {
                    article_id: 0,
                    url_id: 1,
                    url: "https://example.com/same".into(),
                    depth: 0,
                    priority: Priority::default(),
                    discovered_from: None,
                },
            };
            let file = fs::File::create(path)?;
            serde_json::to_writer(file, &page)?;
            Ok(())
        });

        let path1 = archiver.store_page(&mock1).unwrap();

        let mut mock2 = MockPageReader::new();
        mock2.expect_current().return_const(Some(snapshot));
        mock2.expect_url().return_const(url.to_owned());
        mock2.expect_write().returning(|_| Ok(()));

        let path2 = archiver.store_page(&mock2).unwrap();

        assert_eq!(path1, path2);

        cleanup(&base);
    }

    #[test]
    fn test_sanitization_applied() {
        let base = test_dir();
        let archiver = FullPathArchiver::for_path(base.clone());

        let url = "https://example.com/a<>b/c:d";
        let snapshot = make_snapshot(url, 1700000000);

        let mut mock = MockPageReader::new();
        mock.expect_current().return_const(Some(snapshot));
        mock.expect_url().return_const(url.to_owned());

        let path = archiver.generate_filename(&mock).unwrap();
        let path_str = path.to_string_lossy();

        assert!(!path_str.contains("<"));
        assert!(!path_str.contains(">"));
        assert!(!path_str.contains(":"));

        cleanup(&base);
    }

    #[test]
    fn test_hash_sharding_present() {
        let base = test_dir();
        let archiver = FullPathArchiver::for_path(base.clone());

        let url = "https://example.com/shard";
        let snapshot = make_snapshot(url, 1700000000);

        let mut mock = MockPageReader::new();
        mock.expect_current().return_const(Some(snapshot));
        mock.expect_url().return_const(url.to_owned());

        let path = archiver.generate_filename(&mock).unwrap();

        let components: Vec<_> = path.components().collect();

        // Expect: archive / domain / ... / xx / yy / file
        assert!(components.len() >= 5);

        cleanup(&base);
    }

    fn ctx() -> FullPathArchiver {
        FullPathArchiver::for_path("/archive".into())
    }

    #[test]
    fn test_basic_url() {
        let ctx = ctx();
        let path = ctx
            .canonical_filename("https://example.com/a/b/c", 1700000000)
            .unwrap();

        let path_str = path.to_string_lossy();
        assert!(path_str.contains("/archive/example.com/a/b/c/"));
        assert!(path_str.ends_with("_2023-11.json"));
    }

    #[test]
    fn test_root_path_url() {
        let ctx = ctx();
        let path = ctx
            .canonical_filename("https://example.com", 1700000000)
            .unwrap();

        let path_str = path.to_string_lossy();

        // No path segments → slug should be "_"
        assert!(path_str.contains("/archive/example.com/"));
        assert!(path_str.contains("/_/"));
        assert!(path_str.ends_with("_2023-11.json"));
    }

    #[test]
    fn test_trailing_slash() {
        let ctx = ctx();
        let path = ctx
            .canonical_filename("https://example.com/a/b/", 1700000000)
            .unwrap();

        let path_str = path.to_string_lossy();

        // last segment is empty → slug should fallback to "_"
        assert!(path_str.contains("/archive/example.com/a/b/"));
        assert!(path_str.contains("/_/"));
        assert!(path_str.ends_with("_2023-11.json"));
    }

    #[test]
    fn test_sanitization_removes_bad_chars() {
        let ctx = ctx();
        let path = ctx
            .canonical_filename("https://example.com/a/$$$/c!!!", 1700000000)
            .unwrap();

        let path_str = path.to_string_lossy();

        // ensure sanitized segments don't include illegal chars
        assert!(!path_str.contains("$$$"));
        assert!(!path_str.contains("!!!"));
    }

    #[test]
    fn test_unknown_domain() {
        let ctx = ctx();
        let path = ctx
            .canonical_filename("file:///some/path", 1700000000)
            .unwrap();

        let path_str = path.to_string_lossy();

        assert!(path_str.contains("/unknown/"));
    }

    #[test]
    fn test_invalid_url() {
        let ctx = ctx();
        let err = ctx.canonical_filename("not a url", 1700000000);

        assert!(err.is_err());
        let msg = format!("{:?}", err.err().unwrap());
        assert!(msg.contains("Invalid URL"));
    }

    #[test]
    fn test_invalid_timestamp() {
        let ctx = ctx();

        // invalid timestamp (out of range)
        let err = ctx.canonical_filename("https://example.com", i64::MAX);

        assert!(err.is_err());
        let msg = format!("{:?}", err.err().unwrap());
        assert!(msg.contains("Invalid timestamp"));
    }

    #[test]
    fn test_hash_sharding_structure() {
        let ctx = ctx();
        let path = ctx
            .canonical_filename("https://example.com/a", 1700000000)
            .unwrap();

        let components: Vec<_> = path.components().collect();

        // Expect at least: /archive/domain/.../XX/YY/file.json
        assert!(components.len() >= 5);

        let shard1 = components[components.len() - 3]
            .as_os_str()
            .to_string_lossy();
        let shard2 = components[components.len() - 2]
            .as_os_str()
            .to_string_lossy();

        assert_eq!(shard1.len(), 2);
        assert_eq!(shard2.len(), 2);
    }

    #[test]
    fn test_filename_format_contains_date() {
        let ctx = ctx();
        let path = ctx
            .canonical_filename("https://example.com/a", 1700000000)
            .unwrap();

        let filename = path.file_name().unwrap().to_string_lossy();

        // Expect pattern: hash_slug_YYYY-MM.json
        assert!(filename.contains("_20")); // year prefix
        assert!(filename.contains("-")); // month separator
        assert!(filename.ends_with(".json"));
    }

    #[test]
    fn test_empty_segments_filtered() {
        let ctx = ctx();
        let path = ctx
            .canonical_filename("https://example.com//a///b", 1700000000)
            .unwrap();

        let path_str = path.to_string_lossy();

        // ensure no empty path components (i.e., no "//")
        assert!(!path_str.contains("//"));
    }
}
