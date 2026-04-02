use clap::Parser;
use common::settings::Host;
use figment::{
    Figment,
    providers::{Format as _, Serialized, Yaml},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Config {
    pub archive_dir: String,
    pub hosts: Vec<Host>,
    pub target_dir: String,
    pub update: bool,
    /// Optional filter: only write files for URLs containing this substring
    pub url_filter: Option<String>,
    /// Delete source files after successful rebuild
    pub cleanup: bool,
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

    /// Directory where new document archive should be stored
    #[arg(short, long, help_heading = "Rebuild")]
    #[serde(skip_serializing_if = "Option::is_none")]
    target_dir: Option<String>,

    /// Should changes be applied
    #[arg(short, long, help_heading = "Rebuild")]
    update: bool,

    /// filter: only write files for URLs containing this substring
    #[arg(short('f'), long, help_heading = "Rebuild")]
    #[serde(skip_serializing_if = "Option::is_none")]
    url_filter: Option<String>,

    /// Delete source files after successful rebuild
    #[arg(short, long, help_heading = "Rebuild")]
    cleanup: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            archive_dir: "archive".to_string(),
            hosts: Default::default(),
            target_dir: "rebuilt_archive".to_string(),
            update: false,
            url_filter: None,
            cleanup: false,
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
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_config_default_cleanup_is_false() {
        let config = Config::default();
        assert!(!config.cleanup);
    }

    #[test]
    fn test_config_default_url_filter_is_none() {
        let config = Config::default();
        assert_eq!(config.url_filter, None);
    }

    #[test]
    fn test_config_default_update_is_false() {
        let config = Config::default();
        assert!(!config.update);
    }
}
