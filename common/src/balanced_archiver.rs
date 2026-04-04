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
    types::ExtractedPage,
    url::{hash_url, sanitize},
};

pub struct BalancedArchiver {
    archive_dir: PathBuf,
}

impl BalancedArchiver {
    pub fn new(archive_dir: PathBuf) -> Self {
        Self { archive_dir }
    }
}

impl Archiver for BalancedArchiver {
    fn store_page(&self, page: &dyn PageReader) -> anyhow::Result<PathBuf> {
        let path = self.generate_filename(page)?;
        page.write(&path)?;
        Ok(path)
    }

    fn generate_filename(&self, page: &dyn PageReader) -> anyhow::Result<PathBuf> {
        let current = page
            .current()
            .as_ref()
            .ok_or_else(|| anyhow::Error::msg("Failed to get current snapshot"))?;
        let url_str = &current.task.url;

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

        let (prefix_segments, last_segment) = segments.split_at(segments.len().saturating_sub(1));

        let slug = last_segment
            .first()
            .map(|s| sanitize(s))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "_".to_string());

        // --- Base path ---
        let mut base_path = self.archive_dir.clone();

        let domain = url.domain().unwrap_or("unknown");
        base_path.push(domain);

        for seg in prefix_segments {
            let clean = sanitize(seg);
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
                format!(
                    "{}_{}_{}-{:02}.json",
                    hash,
                    slug,
                    datetime.year(),
                    datetime.month()
                )
            } else {
                format!(
                    "{}_{}_{}-{:02}_{}.json",
                    hash,
                    slug,
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
                .and_then(|f| serde_json::from_reader::<_, ExtractedPage>(f).ok())
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
