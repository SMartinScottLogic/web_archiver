use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::settings::CONFIG;
use anyhow::Result;
use common::{settings::Host, types::FetchTask};
use email::{MimeMessage, mimeheaders::MimeContentTypeHeader};
use mailparse::parse_content_disposition;
use rusqlite::{Connection, params};
use scraper::{Html, Selector};
use tracing::{debug, error, info, warn};
use url::Url;
use uuid::Uuid;

use crate::frontier::db::frontier::FrontierDbTrait;

mod single;

pub struct EmailDb {
    pub conn: Arc<Mutex<Connection>>,
}
impl EmailDb {
    pub fn connect(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn reset(&self) -> Result<usize> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let updated = tx
            .execute(
                "UPDATE frontier SET status = 'pending' WHERE status = 'in_progress'",
                params![],
            )
            .inspect_err(|e| error!(?e, "reset failed"))?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn enqueue_email(&self, email_file: &Path) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO emails (filename, created_at) VALUES (?1, strftime('%s','now'))",
            params![email_file.to_string_lossy()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_email_status(&self, id: i64, status: &str) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE emails SET status = ?2 WHERE id = ?1",
            params![id, status],
        )
        .inspect_err(|e| error!(?e, "set email to 'in progress' failed"))?;
        tx.commit()?;

        Ok(())
    }

    pub fn next_email(&self) -> Result<(i64, PathBuf)> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let (id, filename): (i64, String) = tx
            .query_row(
                r#"SELECT id, filename FROM emails WHERE status='pending' LIMIT 1"#,
                [],
                |row: &rusqlite::Row<'_>| Ok((row.get(0)?, row.get(1)?)),
            )
            .inspect_err(|e| error!(?e, "next email failed"))?;
        tx.execute(
            "UPDATE emails SET status = 'in_progress' WHERE id = ?1",
            params![id],
        )
        .inspect_err(|e| error!(?e, "set email to 'in progress' failed"))?;
        tx.commit()?;

        Ok((id, PathBuf::from(filename)))
    }
}

pub struct MailboxPoller<DB>
where
    DB: FrontierDbTrait,
{
    frontier_db: Arc<DB>,
    email_db: Arc<EmailDb>,
}

impl<DB> MailboxPoller<DB>
where
    DB: FrontierDbTrait,
{
    pub fn new(frontier_db: Arc<DB>, email_db: Arc<EmailDb>) -> Self {
        Self {
            frontier_db,
            email_db,
        }
    }

    pub fn poll_all(&self) {
        for mailbox in &CONFIG.get().unwrap().mailboxes {
            let db = self.email_db.clone();
            if let Ok(mut connection) = single::ConnectedImap::connect(db, mailbox)
                .inspect_err(|e| error!(?e, ?mailbox, "failed to connect"))
            {
                let _ = connection
                    .fetch(mailbox)
                    .inspect_err(|e| error!(?e, ?mailbox, "failed to fetch"));
                let _ = connection
                    .logout()
                    .inspect_err(|e| error!(?e, ?mailbox, "failed to logout"));
            }
        }
        while let Ok((id, path)) = self.email_db.next_email() {
            info!(?path, "next email");
            let _ = self
                .process(id, &path)
                .inspect_err(|e| error!(?e, id, ?path, "failed to process"));
        }
    }
}

impl<DB> MailboxPoller<DB>
where
    DB: FrontierDbTrait,
{
    fn process<P>(&self, email_db_id: i64, path: P) -> anyhow::Result<()>
    where
        P: AsRef<Path> + std::fmt::Debug,
    {
        info!(file = ?path, "processing email file");

        let body = std::fs::read_to_string(path)?;

        if let Ok(mime_message) = email::MimeMessage::parse(&body) {
            let children = mime_message
                .children
                .iter()
                .fold(HashMap::new(), organise_children());

            for (cid, (text_type, child)) in children.iter().enumerate() {
                debug!("=== child {} ===", cid);
                debug!("type: {:?}", text_type);
                debug!("{:?}", child);
                debug!("body: {}", child.decoded_body_string().unwrap_or_default());
            }

            if let Some(child) = children.get("html").or_else(|| children.get("plain")) {
                let (_, text_type) = child
                    .headers
                    .get_value::<MimeContentTypeHeader>("Content-Type".into())
                    .unwrap()
                    .content_type;
                let content = child.decoded_body_string().unwrap_or_default();
                debug!(text_type, content, "prefered content");

                match text_type.as_str() {
                    "html" => process_urls_from_html(&content, |url| self.enqueue_if_wanted(url)),
                    "text" => {}
                    s => panic!("Unexpected text type: {}", s),
                };
            }
        }

        self.email_db.set_email_status(email_db_id, "processed")?;

        Ok(())
    }

    fn enqueue_if_wanted(&self, url: Url) {
        if let Some(domain) = url.domain() {
            error!(url = url.as_str(), "email url found");
            // Lookup Host array for use_playwright
            let use_playwright = get_matching_domains(&CONFIG.get().unwrap().hosts, domain)
                .iter()
                .any(|host| host.use_playwright);
            let task = FetchTask {
                article_id: 0,
                url_id: 0,
                url: url.as_str().to_owned(),
                depth: 0,
                priority: Default::default(),
                discovered_from: None,
                use_playwright,
            };
            let _ = self
                .frontier_db
                .enqueue_batch(&[task], false)
                .inspect_err(|e| error!(?url, ?e, "Failed to enqueue url from email"));
        }
    }
}

fn organise_children<'a>()
-> impl Fn(HashMap<String, &'a MimeMessage>, &'a MimeMessage) -> HashMap<String, &'a MimeMessage> {
    let media_archive_dir = PathBuf::from(&CONFIG.get().unwrap().archive_dir).join("media");
    move |mut acc, child| {
        if let Ok(content_type) = child
            .headers
            .get_value::<MimeContentTypeHeader>("Content-Type".into())
        {
            match content_type.content_type {
                (major, minor) if major == "text" => {
                    acc.insert(minor, child);
                }
                (major, minor) => {
                    // Save non-text children
                    debug!(major, minor, ?child, "non text child");
                    save_child(&media_archive_dir, child);
                }
            };
        } else {
            error!(?child, "No content type");
        }
        acc
    }
}
fn process_urls_from_html<F>(content: &str, mut f: F)
where
    F: FnMut(url::Url),
{
    let document = Html::parse_document(content);
    // Extract <a href> links
    let selector = Selector::parse("a[href]").unwrap();

    for link in document
        .select(&selector)
        .filter_map(|element| element.value().attr("href"))
        .flat_map(|href| url::Url::parse(href)
            .inspect_err(|e| error!(?href, ?e, "Failed to parse")))
    {
        f(link);
    }
}

fn save_child<P>(media_archive_dir: P, child: &MimeMessage)
where
    P: AsRef<Path> + std::fmt::Debug,
{
    let _ = std::fs::create_dir_all(&media_archive_dir)
        .inspect_err(|e| error!(?e, ?media_archive_dir, "failed to create archive"));

    let filename = generate_filename(media_archive_dir, child);

    if let Some(body) = child.decoded_body_bytes()
        && !body.is_empty()
    {
        warn!(?filename, "write non-text");
        let _ = std::fs::write(&filename, body).inspect_err(|e| error!(?e, "failed to write"));
    }
}

fn generate_filename<P>(archive_dir: P, child: &MimeMessage) -> PathBuf
where
    P: AsRef<Path>,
{
    if let Ok(content_disposition) = child
        .headers
        .get_value::<String>("Content-Disposition".into())
    {
        let content_disposition = parse_content_disposition(&content_disposition);
        debug!(?content_disposition.disposition, ?content_disposition.params, "content disposition");
        if let Some((_key, name)) = content_disposition
            .params
            .iter()
            .find(|(k, _value)| k.eq_ignore_ascii_case("filename"))
        {
            let filename = archive_dir.as_ref().join(name);
            if !filename.exists() {
                return filename;
            }
        }
    }

    if let Ok(content_type_header) = child
        .headers
        .get_value::<MimeContentTypeHeader>("Content-Type".into())
    {
        debug!(?content_type_header.content_type, ?content_type_header.params, "content disposition");
        if let Some((_k, name)) = content_type_header
            .params
            .iter()
            .find(|(k, _value)| k.eq_ignore_ascii_case("name"))
        {
            let filename = archive_dir.as_ref().join(name);
            if !filename.exists() {
                return filename;
            }
        }
    }

    loop {
        let uuid = Uuid::new_v4();
        let filename = archive_dir.as_ref().join(format!("{}.raw.child", uuid));
        if !filename.exists() {
            break filename;
        }
    }
}

fn get_matching_domains<'a>(hosts: &'a [Host], domain: &str) -> Vec<&'a Host> {
    hosts
        .iter()
        .filter(|&host| host.domains.iter().any(|d| d == domain))
        .collect()
}
