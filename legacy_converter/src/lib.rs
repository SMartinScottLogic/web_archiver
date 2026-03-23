use std::collections::HashSet;

use anyhow::Result;
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use map_macro::hash_map;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tracing::debug;

use common::types::{ExtractedPage, FetchTask, PageMetadata};

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
pub struct LegacyStory {
    #[serde(alias = "_id")]
    pub id: String,
    pub story_id: String,
    pub page: usize,
    pub uri: String,
    pub story: Vec<String>,
    pub keywords: Vec<String>,
    pub title: String,
    pub author: String,
    #[serde(skip)]
    pub links: HashSet<String>,
}

pub fn parse_unambiguous_date(s: &str) -> Result<u64, String> {
    let formats = ["%Y-%m-%d", "%d/%m/%Y", "%m/%d/%Y", "%Y-%m-%d %H:%M:%S"];

    let mut results = Vec::new();

    for fmt in formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            results.push(Utc.from_utc_datetime(&dt).timestamp());
            continue;
        }

        if let Ok(date) = NaiveDate::parse_from_str(s, fmt)
            && let Some(dt) = date.and_hms_opt(0, 0, 0)
        {
            results.push(Utc.from_utc_datetime(&dt).timestamp());
        }
    }

    results.sort();
    results.dedup();

    match results.len() {
        0 => Err(format!(
            "Invalid date '{}'. Use YYYY-MM-DD (e.g. 2026-03-13)",
            s
        )),
        1 => Ok(results[0] as u64),
        _ => Err(format!("Ambiguous date '{}'. Use YYYY-MM-DD", s)),
    }
}

fn convert(path: &Path, fetch_time: u64) -> Result<ExtractedPage, anyhow::Error> {
    debug!(?path, "convert file");
    let text = fs::read_to_string(path)?;
    let content: LegacyStory = serde_json::from_str(&text)?;
    let markdown = content.story.join("\n\n");
    let url = content.uri;

    // TODO Retrieve matching record from DB (if present), and use to populate - or don't bother as legacy?
    let url_id = 0;
    let discovered_from = None;

    let page = ExtractedPage {
        task: FetchTask {
            url_id,
            url,
            depth: u32::MAX,
            priority: 0,
            discovered_from,
        },
        content_markdown: Some(markdown),
        links: content.links.into_iter().collect(),
        metadata: Some(PageMetadata {
            status_code: 200,
            content_type: None,
            fetch_time,
            title: Some(content.title),
            document_metadata: Some(vec![
                hash_map! {"keywords".to_string() => content.keywords.join(",")},
            ]),
        }),
    };
    Ok(page)
}

pub fn store_file(path: &Path, fetch_time: u64, delete_source: bool) -> Result<()> {
    let converted = convert(path, fetch_time)?;
    // Save new file
    let fetch_time = DateTime::<Utc>::from_timestamp(fetch_time.try_into().unwrap(), 0).unwrap();
    common::url::store_page(&converted, fetch_time)?;
    // TODO Add to DB?
    if delete_source {
        fs::remove_file(path)?;
        debug!(?path, "Removed source file")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_unambiguous_date() {
        assert!(parse_unambiguous_date("2026-03-13").is_ok());
        assert!(parse_unambiguous_date("13/03/2026").is_ok());
        assert!(parse_unambiguous_date("03/13/2026").is_ok());
        assert!(parse_unambiguous_date("01/02/2026").is_err());
    }
}
