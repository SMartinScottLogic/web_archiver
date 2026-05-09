use anyhow::{Context as _, Result};
use clap::Parser;
use common::settings::{CONFIG_FILE, Mailbox};
use figment::{
    Figment,
    providers::{Format, Serialized, Yaml},
};
use imap::Session;
use native_tls::{TlsConnector, TlsStream};
use serde::{Deserialize, Serialize};
use std::{net::TcpStream, path::Path};
use tracing::{debug, info, level_filters::LevelFilter};
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};

type ImapSession = Session<TlsStream<TcpStream>>;

pub struct ImapProcessor {
    name: String,
    session: ImapSession,
}
impl ImapProcessor {
    pub fn connect(params: Mailbox) -> Result<Self> {
        let tls = TlsConnector::builder()
            .build()
            .context("failed building TLS connector")?;
        let client = imap::connect((params.server(), params.port()), params.server(), &tls)
            .context("failed connecting to IMAP server")?;

        let session = client
            .login(params.email(), params.password())
            .map_err(|e| e.0)
            .context("failed logging into Gmail IMAP")?;

        Ok(Self {
            name: params.name(),
            session,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        info!("Listing mailboxes for {}", self.name);
        for s in &self.session.list(Some("*/*"), Some("*"))? {
            info!("  {:?}", s.name());
        }

        info!("done");
        Ok(())
    }
}

#[derive(Parser, Debug, Serialize)]
#[command(rename_all = "kebab-case")]
#[serde(rename_all = "snake_case")]
#[command(author, version, about, long_about = None)]
struct Args {}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub struct Config {
    pub mailboxes: Vec<Mailbox>,
}

impl Config {
    pub fn file<P>(path: P) -> anyhow::Result<Self>
    where
        P: AsRef<Path>,
    {
        let cli = Args::parse();
        let config: Self = Figment::new()
            .merge(Serialized::defaults(Config::default()))
            .merge(Yaml::file(path))
            .merge(Serialized::from(cli, "default"))
            .extract()?;
        Ok(config)
    }
}

fn setup_logging() {
    // Initialize logging
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_span_events(FmtSpan::NONE)
        .init();
}

fn main() -> Result<()> {
    setup_logging();

    let config =
        Config::file(CONFIG_FILE).unwrap_or_else(|_| panic!("Failed to load {}", CONFIG_FILE));

    debug!("config: {:?}", config);

    for mailbox in config.mailboxes {
        let mut imap_processor = ImapProcessor::connect(mailbox)?;
        imap_processor.run()?;
    }
    Ok(())
}
