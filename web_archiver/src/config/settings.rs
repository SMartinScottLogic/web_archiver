use clap::Parser;
use figment::{
    Figment,
    providers::{Format as _, Serialized, Yaml},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Config {
    pub hosts: Vec<Host>,
    pub workers: usize,
    pub seed_urls: Vec<String>,
    pub noop_delay_millis: u64,
    pub user_agent: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Host {
    pub name: String,
    pub domains: Vec<String>,
}

/// Command line arguments
#[derive(Parser, Debug, Serialize)]
#[command(rename_all = "kebab-case")]
#[serde(rename_all = "snake_case")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Delay in ms for frontier manager idle loop
    #[arg(short, long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    noop_delay_millis: Option<u64>,

    /// Number of concurrent fetch workers
    #[arg(short, long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    workers: Option<usize>,

    /// User Agent to supply for fetches
    #[arg(short, long)]
    #[serde(skip_serializing_if = "Option::is_none")]
    user_agent: Option<String>,

    /// Crawl seeds
    #[serde(skip_serializing_if = "Option::is_none")]
    seed_urls: Option<Vec<String>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hosts: Default::default(),
            workers: 1,
            seed_urls: Default::default(),
            noop_delay_millis: 500,
            user_agent: "Week1Crawler/0.1".to_string(),
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
}
