use crate::settings::CONFIG;
use anyhow::{Context as _, Result};
use common::settings::Mailbox;
use imap::Session;
use mailparse::{MailHeaderMap, parse_mail};
use native_tls::{TlsConnector, TlsStream};
use std::{
    fmt::Display,
    net::TcpStream,
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::mail::EmailDb;

type ImapSession = Session<TlsStream<TcpStream>>;

pub struct ConnectedImap {
    db: Arc<EmailDb>,
    email_archive_dir: PathBuf,
    session: ImapSession,
    mailbox_name: String,
    dry_run: bool,
}
impl Display for ConnectedImap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ConnectedImap({})", self.mailbox_name)
    }
}
impl ConnectedImap {
    pub fn connect(db: Arc<EmailDb>, mailbox: &Mailbox) -> Result<Self> {
        match &mailbox.r#box {
            None => Err(anyhow::Error::msg(format!(
                "No mailbox supplied for {}",
                mailbox.name
            ))),
            Some(mailbox_name) => {
                let tls = TlsConnector::builder()
                    .build()
                    .context("failed building TLS connector")?;
                let client =
                    imap::connect((mailbox.server(), mailbox.port()), mailbox.server(), &tls)
                        .context("failed connecting to IMAP server")?;

                let session = client
                    .login(mailbox.email(), mailbox.password())
                    .map_err(|e| e.0)
                    .context("failed logging into Gmail IMAP")?;
                let email_archive_dir = PathBuf::from(&CONFIG.get().unwrap().archive_dir)
                    .join("email")
                    .join(mailbox.name());
                std::fs::create_dir_all(&email_archive_dir).context("create archive path")?;
                let dry_run = mailbox.dry_run;

                Ok(Self {
                    db,
                    email_archive_dir,
                    session,
                    mailbox_name: mailbox_name.to_owned(),
                    dry_run,
                })
            }
        }
    }

    pub fn logout(mut self) -> anyhow::Result<()> {
        self.session.logout().context("logout")
    }
}

impl ConnectedImap {
    pub fn fetch(&mut self, mailbox: &Mailbox) -> Result<()> {
        self.session
            .select(&self.mailbox_name)
            .context("select mailbox")?;

        info!("Connected to {} on {}", self.mailbox_name, mailbox.name);

        let ids = self.fetch_message_ids()?;

        if ids.is_empty() {
            info!("no draft messages found");
            return Ok(());
        }

        info!("found {} draft messages", ids.len());

        for id in ids {
            match self.process_message(id) {
                Ok(_) => debug!("processed message {}", id),
                Err(e) => warn!("failed processing {}: {:?}", id, e),
            }
        }

        let expunged = self
            .session
            .expunge()
            .context("failed expunging messages")?;
        info!("Expunged: {:?}", expunged);
        info!("done");

        Ok(())
    }
}

impl ConnectedImap {
    fn fetch_message_ids(&mut self) -> Result<Vec<u32>> {
        let messages = self
            .session
            .search("ALL")
            .context("failed searching Drafts mailbox")?;

        let mut messages: Vec<u32> = messages.into_iter().collect();
        messages.sort();
        Ok(messages)
    }

    fn process_message(&mut self, id: u32) -> Result<()> {
        info!(id, %self, "process message");

        let fetches = self
            .session
            .fetch(id.to_string(), "RFC822")
            .context("failed fetching message")?;

        let fetch = fetches.iter().next().context("message not found")?;

        let body = fetch.body().context("message body missing")?;

        let parsed = parse_mail(body).context("failed parsing email")?;

        let subject = parsed
            .headers
            .get_first_value("Subject")
            .unwrap_or_else(|| "(no subject)".to_string());

        debug!("message {} subject: {}", id, subject);

        let filename = generate_filename(&self.email_archive_dir);
        std::fs::write(&filename, body).context("write message")?;

        self.db.enqueue_email(&filename)?;

        if should_delete(&subject) {
            if self.dry_run {
                warn!("[dry-run] would delete message {}", id);
            } else {
                self.delete_message(id)?;
                debug!("deleted message {}", id);
            }
        } else {
            debug!("keeping message {}", id);
        }
        Ok(())
    }
}

impl ConnectedImap {
    fn delete_message(&mut self, id: u32) -> Result<()> {
        // mark deleted
        self.session
            .store(id.to_string(), "+FLAGS (\\Deleted)")
            .context("failed marking message deleted")?;
        Ok(())
    }
}

fn generate_filename(archive: &Path) -> PathBuf {
    loop {
        let uuid = Uuid::new_v4();
        let filename = archive.join(format!("{}.raw", uuid));
        if !filename.exists() {
            break filename;
        }
    }
}

/// Replace with real rules, e.g.:
/// - subject matches automation marker
/// - stale generated drafts
/// - old workflow leftovers
///
/// Example:
///
/// if subject.contains("[AUTO-DRAFT]") {
///     return true;
/// }
///
fn should_delete(_subject: &str) -> bool {
    true
}
