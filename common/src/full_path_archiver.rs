use std::{
    fs::{File, create_dir_all},
    marker::PhantomData,
    path::PathBuf,
};

use anyhow::Context as _;
use chrono::{DateTime, Datelike as _, Utc};
use url::Url;

use crate::{
    Archiver,
    types::ExtractedPage,
    url::{hash_url, sanitize},
};

pub struct FullPathArchiver {
    _data: PhantomData<usize>,
}

impl FullPathArchiver {
    pub fn new() -> Self {
        Self { _data: PhantomData }
    }
}

impl Default for FullPathArchiver {
    fn default() -> Self {
        Self::new()
    }
}

impl Archiver for FullPathArchiver {
    fn store_page(&self, page: &ExtractedPage) -> anyhow::Result<PathBuf> {
        let path = self.generate_filename(page)?;
        page.write_page(&path)?;
        Ok(path)
    }

    fn generate_filename(&self, page: &ExtractedPage) -> anyhow::Result<PathBuf> {
        let url_str = &page.task.url;

        let url = Url::parse(url_str).with_context(|| format!("Invalid URL: {}", url_str))?;

        // --- Time ---
        let datetime = page
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
        let mut base_path = PathBuf::from("archive");

        let domain = url.domain().unwrap_or("unknown");
        base_path.push(domain);

        for seg in segments {
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
