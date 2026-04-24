use chrono::{NaiveDate, NaiveDateTime, TimeZone as _, Utc};
use clap::Parser;
use common::settings::Host;
use figment::{
    Figment,
    providers::{Format as _, Serialized, Yaml},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Config {
    pub archive_dir: String,
    pub archive_time: i64,
    pub hosts: Vec<Host>,
    pub workers: usize,
    pub seed_urls: Vec<String>,
    pub noop_delay_millis: u64,
    pub user_agent: String,
    pub db: String,
}

/// Command line arguments
#[derive(Parser, Debug, Serialize)]
#[command(rename_all = "kebab-case")]
#[serde(rename_all = "snake_case")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Directory where document archive is stored
    #[arg(short, long, help_heading = "Archive")]
    #[serde(skip_serializing_if = "Option::is_none")]
    archive_dir: Option<String>,

    /// Time when the crawl should be associated with
    #[arg(short('t'), long, help_heading = "Archive", value_parser = parse_unambiguous_date)]
    #[serde(skip_serializing_if = "Option::is_none")]
    archive_time: Option<i64>,

    /// Delay in ms for frontier manager idle loop
    #[arg(short, long, help_heading = "Crawl")]
    #[serde(skip_serializing_if = "Option::is_none")]
    noop_delay_millis: Option<u64>,

    /// Number of concurrent fetch workers
    #[arg(short, long, help_heading = "Crawl")]
    #[serde(skip_serializing_if = "Option::is_none")]
    workers: Option<usize>,

    /// User Agent to supply for fetches
    #[arg(short, long, help_heading = "Crawl")]
    #[serde(skip_serializing_if = "Option::is_none")]
    user_agent: Option<String>,

    /// Crawl seeds
    #[serde(skip_serializing_if = "Option::is_none")]
    seed_urls: Option<Vec<String>>,

    /// Database for queue and metadata store
    #[arg(short, long, help_heading = "Archive")]
    #[serde(skip_serializing_if = "Option::is_none")]
    db: Option<String>,
}

#[allow(dead_code)]
pub fn parse_unambiguous_date(s: &str) -> Result<i64, String> {
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
        1 => Ok(results[0]),
        _ => Err(format!("Ambiguous date '{}'. Use YYYY-MM-DD", s)),
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            archive_dir: "archive".to_string(),
            archive_time: chrono::Utc::now().timestamp(),
            hosts: Default::default(),
            workers: 1,
            seed_urls: Default::default(),
            noop_delay_millis: 500,
            user_agent: "Week1Crawler/0.1".to_string(),
            db: "crawler.db".to_string(),
        }
    }
}
impl Config {
    pub fn file(path: &str) -> anyhow::Result<Self> {
        let cli = Args::parse();
        let config: Self = Figment::new()
            .merge(Serialized::defaults(Config::default()))
            .merge(Yaml::file(path))
            .merge(Serialized::from(cli, "default"))
            .extract()?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_load_from_file() {
        let yaml = "hosts:\n  - name: Foo\n    domains: [foo.com]\nworkers: 2\nseed_urls:\n  - http://foo.com\n";
        let path = "test_config.yaml";
        let mut file = File::create(path).unwrap();
        file.write_all(yaml.as_bytes()).unwrap();
        let config = Config::file(path).unwrap();
        assert_eq!(config.hosts.len(), 1);
        assert_eq!(config.hosts[0].name, "Foo");
        assert_eq!(config.hosts[0].domains, vec!["foo.com"]);
        assert_eq!(config.workers, 2);
        assert_eq!(config.seed_urls, vec!["http://foo.com".to_string()]);
        std::fs::remove_file(path).unwrap();
    }

    // ----------------------------
    // parse_unambiguous_date tests
    // ----------------------------

    #[test]
    fn test_parse_unambiguous_date_valid_formats() {
        assert!(parse_unambiguous_date("2026-03-13").is_ok());
        assert!(parse_unambiguous_date("13/03/2026").is_ok());
        assert!(parse_unambiguous_date("03/13/2026").is_ok());
        assert!(parse_unambiguous_date("01/02/2026").is_err());
        assert!(parse_unambiguous_date("2026-03-13 12:00:00").is_ok());
    }

    #[test]
    fn test_parse_unambiguous_date_ambiguous() {
        let err = parse_unambiguous_date("01/02/2026").unwrap_err();
        assert!(err.contains("Ambiguous"));
    }

    #[test]
    fn test_parse_unambiguous_date_invalid() {
        let err = parse_unambiguous_date("not-a-date").unwrap_err();
        assert!(err.contains("Invalid"));
    }

    #[test]
    fn test_parse_unambiguous_date_consistent_timestamp() {
        let ts1 = parse_unambiguous_date("2026-03-13").unwrap();
        let ts2 = parse_unambiguous_date("2026-03-13 00:00:00").unwrap();
        assert_eq!(ts1, ts2);
    }
}
