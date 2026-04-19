use serde::{Deserialize, Serialize};

pub static CONFIG_FILE: &str = "config.yaml";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Host {
    pub name: String,
    pub domains: Vec<String>,
    #[serde(default)]
    pub pages: PageType,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub enum PageType {
    #[serde(alias = "none")]
    #[default]
    None,
    #[serde(alias = "query-param")]
    QueryParam { key: String, default: usize },
}
