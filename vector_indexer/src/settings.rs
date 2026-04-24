use clap::Parser;
use figment::{
    Figment,
    providers::{Format as _, Serialized, Yaml},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Config {
    pub archive_dir: String,
    pub collection: String,
}

#[derive(Parser, Debug, Serialize)]
#[command(rename_all = "kebab-case")]
#[serde(rename_all = "snake_case")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Directory where existing document archive is stored
    #[arg(short, long, help_heading = "Archive")]
    #[serde(skip_serializing_if = "Option::is_none")]
    archive_dir: Option<String>,

    /// Delete source files after successful rebuild
    #[arg(short, long, help_heading = "Vector")]
    #[serde(skip_serializing_if = "Option::is_none")]
    collection: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            archive_dir: "archive".to_string(),
            collection: "archive".to_string(),
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
        assert_eq!(config.archive_dir, "archive");
        assert_eq!(config.collection, "archive");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_config_default_archive_dir() {
        let config = Config::default();
        assert_eq!(config.archive_dir, "archive");
    }

    #[test]
    fn test_config_default_collection() {
        let config = Config::default();
        assert_eq!(config.collection, "archive");
    }
}
