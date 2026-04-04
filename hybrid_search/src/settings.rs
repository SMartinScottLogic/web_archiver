use clap::Parser;
use figment::{
    providers::{Format as _, Serialized, Yaml},
    Figment,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Config {
    pub collection: String,
    pub limit: u64,
    pub source: Option<String>,
    pub query: String,
}

#[derive(Parser, Debug, Serialize)]
#[command(rename_all = "kebab-case")]
#[serde(rename_all = "snake_case")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Collection to perform query against
    #[arg(short, long, help_heading = "Vector")]
    #[serde(skip_serializing_if = "Option::is_none")]
    collection: Option<String>,

    /// Number of results wanted
    #[arg(long, short, help_heading = "Search")]
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u64>,

    /// Optional source filter
    #[arg(long, help_heading = "Search")]
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,

    /// Query text
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            collection: "archive".to_string(),
            limit: 5,
            source: None,
            query: "hello world".to_string(),
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
        assert_eq!(config.limit, 5);
        assert_eq!(config.collection, "archive");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_config_default_collection() {
        let config = Config::default();
        assert_eq!(config.collection, "archive");
    }

    #[test]
    fn test_config_default_limit() {
        let config = Config::default();
        assert_eq!(config.limit, 5);
    }
}
