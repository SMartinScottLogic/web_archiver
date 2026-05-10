use serde::{Deserialize, Serialize};

pub static CONFIG_FILE: &str = "config.yaml";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Host {
    pub name: String,
    pub domains: Vec<String>,
    #[serde(default)]
    pub pages: PageType,
    #[serde(default)]
    pub use_playwright: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub enum PageType {
    #[serde(alias = "none")]
    #[default]
    None,
    #[serde(alias = "query-param")]
    QueryParam { key: String, default: usize },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Mailbox {
    pub name: String,
    #[serde(default)]
    pub mode: MailboxType,
    pub server: String,
    pub port: u16,
    pub email: String,
    pub password: String,
    pub r#box: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub enum MailboxType {
    #[serde(alias = "none")]
    #[default]
    None,
    #[serde(alias = "imap")]
    Imap,
}

impl Mailbox {
    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn server(&self) -> String {
        self.server.clone()
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn email(&self) -> String {
        self.email.clone()
    }

    pub fn password(&self) -> String {
        self.password.clone()
    }
}
