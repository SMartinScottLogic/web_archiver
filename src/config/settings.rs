use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize)]
pub struct DomainConfig {
    pub allowed_domains: Vec<String>,
    pub workers: Option<usize>,
    pub seed_urls: Option<Vec<String>>,
}

impl DomainConfig {
    pub fn load_from_file(path: &str) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: DomainConfig = serde_yaml::from_str(&content)?;
        Ok(config)
    }
}
