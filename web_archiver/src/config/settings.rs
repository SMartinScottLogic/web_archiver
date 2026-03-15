use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize)]
pub struct DomainConfig {
    pub hosts: Vec<Host>,
    pub workers: Option<usize>,
    pub seed_urls: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct Host {
    pub name: String,
    pub domains: Vec<String>,
}

impl DomainConfig {
    pub fn load_from_file(path: &str) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: DomainConfig = serde_yaml::from_str(&content)?;
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
        let config = DomainConfig::load_from_file(path).unwrap();
        assert_eq!(config.hosts.len(), 1);
        assert_eq!(config.hosts[0].name, "Foo");
        assert_eq!(config.hosts[0].domains, vec!["foo.com"]);
        assert_eq!(config.workers, Some(2));
        assert_eq!(config.seed_urls, Some(vec!["http://foo.com".to_string()]));
        std::fs::remove_file(path).unwrap();
    }
}
