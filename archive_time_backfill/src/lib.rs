use std::{collections::BTreeMap, fs::File, io::BufReader, path::Path, time::Duration};

use anyhow::{Context, bail};
use common::{historical::HistoricalPage, url::remove_pagination_params};
use indicatif::{ProgressBar, ProgressStyle};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction};
use walkdir::WalkDir;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct BackfillReport {
    pub files_scanned: usize,
    pub skipped_files: Vec<SkippedArchiveFile>,
    pub archive_articles: usize,
    pub matched_articles: usize,
    pub unmatched_urls: Vec<String>,
    pub frontier_rows_to_update: usize,
    pub frontier_rows_updated: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SkippedArchiveFile {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SkippedArchiveFileSummary {
    pub count: usize,
    pub reason: String,
    pub example_path: String,
}

pub fn summarize_skipped_archive_files(
    skipped_files: &[SkippedArchiveFile],
) -> Vec<SkippedArchiveFileSummary> {
    let mut summaries = BTreeMap::<String, SkippedArchiveFileSummary>::new();
    for skipped in skipped_files {
        summaries
            .entry(skipped.reason.clone())
            .and_modify(|summary| summary.count += 1)
            .or_insert_with(|| SkippedArchiveFileSummary {
                count: 1,
                reason: skipped.reason.clone(),
                example_path: skipped.path.clone(),
            });
    }
    summaries.into_values().collect()
}

#[derive(Debug, PartialEq, Eq)]
struct ArchiveTimestamp {
    url: String,
    fetch_time: i64,
}

pub fn backfill_archive_timestamps(
    archive_dir: &Path,
    database_path: &Path,
    apply: bool,
) -> anyhow::Result<BackfillReport> {
    if !archive_dir.is_dir() {
        bail!(
            "archive directory does not exist or is not a directory: {}",
            archive_dir.display()
        );
    }
    if !database_path.is_file() {
        bail!(
            "database file does not exist or is not a file: {}",
            database_path.display()
        );
    }

    let scan_progress = ProgressBar::new_spinner();
    scan_progress.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg} [{elapsed_precise}]")
            .expect("static progress template is valid"),
    );
    scan_progress.enable_steady_tick(Duration::from_millis(100));
    scan_progress.set_message("Scanning archive JSON files");
    let (files_scanned, skipped_files, archive_timestamps) =
        scan_archive(archive_dir, &scan_progress)?;
    scan_progress.finish_with_message(format!(
        "Archive scan complete: {files_scanned} files, {} articles, {} skipped",
        archive_timestamps.len(),
        skipped_files.len()
    ));
    let mode = if apply {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let mut conn =
        Connection::open_with_flags(database_path, mode | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .with_context(|| format!("open database {}", database_path.display()))?;
    let tx = conn.transaction().context("start backfill transaction")?;
    let report = update_database(&tx, files_scanned, skipped_files, archive_timestamps, apply)?;
    if apply {
        tx.commit().context("commit backfill transaction")?;
    }
    Ok(report)
}

fn scan_archive(
    archive_dir: &Path,
    progress: &ProgressBar,
) -> anyhow::Result<(usize, Vec<SkippedArchiveFile>, Vec<ArchiveTimestamp>)> {
    let mut files_scanned = 0;
    let mut skipped_files = Vec::new();
    let mut timestamps_by_url = BTreeMap::<String, i64>::new();

    for entry in WalkDir::new(archive_dir) {
        let entry =
            entry.with_context(|| format!("walk archive directory {}", archive_dir.display()))?;
        if !entry.file_type().is_file()
            || entry
                .path()
                .extension()
                .is_none_or(|extension| extension != "json")
        {
            continue;
        }

        files_scanned += 1;
        let path = entry.path();
        let parsed = File::open(path)
            .with_context(|| format!("open archive file {}", path.display()))
            .and_then(|file| {
                serde_json::from_reader::<_, HistoricalPage>(BufReader::new(file))
                    .with_context(|| format!("parse archive file {}", path.display()))
            })
            .and_then(|page| {
                let fetch_time = latest_fetch_time(&page)
                    .with_context(|| format!("find fetch timestamp in {}", path.display()))?;
                Ok((page, fetch_time))
            });
        let (page, fetch_time) = match parsed {
            Ok(parsed) => parsed,
            Err(error) => {
                skipped_files.push(SkippedArchiveFile {
                    path: path.display().to_string(),
                    reason: normalize_error_reason(&error),
                });
                progress.inc(1);
                progress.set_message(format!(
                    "Files processed: {files_scanned} | Articles: {} | Skipped: {}",
                    timestamps_by_url.len(),
                    skipped_files.len()
                ));
                continue;
            }
        };
        let url = remove_pagination_params(&page.task.url);
        if url.trim().is_empty() {
            skipped_files.push(SkippedArchiveFile {
                path: path.display().to_string(),
                reason: "archive record has an empty task URL".to_owned(),
            });
            progress.inc(1);
            progress.set_message(format!(
                "Files processed: {files_scanned} | Articles: {} | Skipped: {}",
                timestamps_by_url.len(),
                skipped_files.len()
            ));
            continue;
        }

        timestamps_by_url
            .entry(url)
            .and_modify(|existing| *existing = (*existing).max(fetch_time))
            .or_insert(fetch_time);
        progress.inc(1);
        progress.set_message(format!(
            "Files processed: {files_scanned} | Articles: {} | Skipped: {}",
            timestamps_by_url.len(),
            skipped_files.len()
        ));
    }

    let timestamps = timestamps_by_url
        .into_iter()
        .map(|(url, fetch_time)| ArchiveTimestamp { url, fetch_time })
        .collect();
    Ok((files_scanned, skipped_files, timestamps))
}

fn normalize_error_reason(error: &anyhow::Error) -> String {
    let message = error.root_cause().to_string();
    let Some((reason, location)) = message.split_once(" at line ") else {
        return message;
    };
    let Some((line, column)) = location.split_once(" column ") else {
        return message;
    };
    if line.parse::<u64>().is_ok() && column.parse::<u64>().is_ok() {
        reason.to_owned()
    } else {
        message
    }
}

fn latest_fetch_time(page: &HistoricalPage) -> anyhow::Result<i64> {
    let timestamp = page
        .current
        .as_ref()
        .and_then(|snapshot| snapshot.metadata.as_ref())
        .map(|metadata| metadata.fetch_time)
        .filter(|&timestamp| timestamp > 0)
        .or_else(|| {
            page.historical_snapshots
                .iter()
                .filter_map(|snapshot| {
                    snapshot
                        .metadata
                        .as_ref()
                        .map(|metadata| metadata.fetch_time)
                })
                .filter(|&timestamp| timestamp > 0)
                .max()
        })
        .or_else(|| {
            page.history
                .iter()
                .copied()
                .filter(|&timestamp| timestamp > 0)
                .max()
        })
        .context("no positive fetch timestamp in current, historical, or history metadata")?;
    i64::try_from(timestamp).context("fetch timestamp exceeds the SQLite integer range")
}

fn update_database(
    tx: &Transaction<'_>,
    files_scanned: usize,
    skipped_files: Vec<SkippedArchiveFile>,
    archive_timestamps: Vec<ArchiveTimestamp>,
    apply: bool,
) -> anyhow::Result<BackfillReport> {
    let progress = ProgressBar::new(archive_timestamps.len() as u64);
    progress.set_style(
        ProgressStyle::with_template(
            "{spinner:.cyan} {msg} [{bar:40.cyan/blue}] {pos}/{len} [{elapsed_precise}]",
        )
        .expect("static progress template is valid")
        .progress_chars("##-"),
    );
    let mut report = BackfillReport {
        files_scanned,
        skipped_files,
        archive_articles: archive_timestamps.len(),
        ..Default::default()
    };

    for archive in archive_timestamps {
        let article_id = find_article_id(tx, &archive.url)?;
        let Some(article_id) = article_id else {
            report.unmatched_urls.push(archive.url);
            advance_database_progress(&progress, &report);
            continue;
        };
        report.matched_articles += 1;

        let frontier_rows = update_article_frontier(tx, &archive, article_id, apply)?;
        report.frontier_rows_to_update += frontier_rows;
        if apply {
            report.frontier_rows_updated += frontier_rows;
        }
        advance_database_progress(&progress, &report);
    }
    progress.finish_with_message(format!(
        "Database matching complete: {} matched, {} unmatched, {} rows to update",
        report.matched_articles,
        report.unmatched_urls.len(),
        report.frontier_rows_to_update
    ));
    Ok(report)
}

fn find_article_id(tx: &Transaction<'_>, url: &str) -> anyhow::Result<Option<i64>> {
    tx.query_row("SELECT id FROM articles WHERE url = ?1", [url], |row| {
        row.get::<_, i64>(0)
    })
    .optional()
    .with_context(|| format!("look up archived URL {url}"))
}

fn update_article_frontier(
    tx: &Transaction<'_>,
    archive: &ArchiveTimestamp,
    article_id: i64,
    apply: bool,
) -> anyhow::Result<usize> {
    if apply {
        return tx
            .execute(
                "UPDATE frontier
                 SET latest_fetch_time = ?1
                 WHERE latest_fetch_time = 0
                   AND url_id IN (SELECT id FROM urls WHERE article_id = ?2)",
                (archive.fetch_time, article_id),
            )
            .with_context(|| format!("update fetch time for {}", archive.url));
    }

    let row_count: i64 = tx
        .query_row(
            "SELECT COUNT(*)
             FROM frontier
             WHERE latest_fetch_time = 0
               AND url_id IN (SELECT id FROM urls WHERE article_id = ?1)",
            [article_id],
            |row| row.get(0),
        )
        .with_context(|| format!("count frontier rows for {}", archive.url))?;
    usize::try_from(row_count).context("frontier row count exceeds the supported range")
}

fn advance_database_progress(progress: &ProgressBar, report: &BackfillReport) {
    progress.inc(1);
    progress.set_message(format!(
        "Articles matched: {} | Unmatched: {} | Rows to update: {}",
        report.matched_articles,
        report.unmatched_urls.len(),
        report.frontier_rows_to_update
    ));
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashSet, VecDeque},
        fs,
        path::Path,
    };

    use common::{
        historical::{
            HistoricalContent, HistoricalContentType, HistoricalPage, HistoricalSnapshot,
        },
        types::{FetchTask, PageMetadata, Priority},
    };
    use rusqlite::Connection;
    use std::path::PathBuf;
    use tempfile::TempDir;

    use super::*;

    fn fetch_task(url: &str) -> FetchTask {
        FetchTask {
            article_id: 1,
            url_id: 1,
            url: url.to_owned(),
            depth: 0,
            priority: Priority::default(),
            discovered_from: None,
            use_playwright: false,
        }
    }

    fn metadata(fetch_time: u64) -> PageMetadata {
        PageMetadata {
            status_code: 200,
            content_type: None,
            fetch_time,
            authors: Vec::new(),
            title: None,
            document_metadata: None,
            json_ld: None,
        }
    }

    fn page(url: &str, current_time: Option<u64>, history: Vec<u64>) -> HistoricalPage {
        HistoricalPage {
            task: fetch_task(url),
            current: current_time.map(|fetch_time| HistoricalSnapshot {
                content_markdown: vec![HistoricalContent {
                    content: HistoricalContentType::Literal("archive".to_owned()),
                    page: 1,
                }],
                links: HashSet::new(),
                metadata: Some(metadata(fetch_time)),
            }),
            historical_snapshots: VecDeque::new(),
            all_links: HashSet::new(),
            history: history.into(),
        }
    }

    fn database() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE articles (id INTEGER PRIMARY KEY, url TEXT NOT NULL UNIQUE);
             CREATE TABLE urls (
                id INTEGER PRIMARY KEY,
                url TEXT NOT NULL,
                article_id INTEGER NOT NULL
             );
             CREATE TABLE frontier (
                url_id INTEGER PRIMARY KEY,
                latest_fetch_time INTEGER NOT NULL DEFAULT 0
             );
             INSERT INTO articles (id, url) VALUES (1, 'https://example.com/story');
             INSERT INTO urls (id, url, article_id) VALUES
                (1, 'https://example.com/story?page=1', 1),
                (2, 'https://example.com/story?page=2', 1);
             INSERT INTO frontier (url_id, latest_fetch_time) VALUES (1, 0), (2, 0);",
        )
        .unwrap();
        drop(conn);
        (dir, path)
    }

    fn write_page(directory: &Path, filename: &str, page: &HistoricalPage) {
        fs::write(directory.join(filename), serde_json::to_vec(page).unwrap()).unwrap();
    }

    fn frontier_fetch_times(database_path: &Path) -> Vec<i64> {
        let conn = Connection::open(database_path).unwrap();
        let mut stmt = conn
            .prepare("SELECT latest_fetch_time FROM frontier ORDER BY url_id")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    #[test]
    fn dry_run_matches_article_without_writing() {
        let archive = tempfile::tempdir().unwrap();
        let (_database_dir, database_path) = database();
        Connection::open(&database_path)
            .unwrap()
            .execute(
                "UPDATE frontier SET latest_fetch_time = 9876 WHERE url_id = 2",
                [],
            )
            .unwrap();
        write_page(
            archive.path(),
            "story.json",
            &page("https://example.com/story?page=2", Some(1234), vec![]),
        );

        let report = backfill_archive_timestamps(archive.path(), &database_path, false).unwrap();

        assert_eq!(report.files_scanned, 1);
        assert_eq!(report.archive_articles, 1);
        assert_eq!(report.matched_articles, 1);
        assert_eq!(report.frontier_rows_to_update, 1);
        assert_eq!(report.frontier_rows_updated, 0);
        assert!(report.unmatched_urls.is_empty());
        assert_eq!(frontier_fetch_times(&database_path), vec![0, 9876]);
    }

    #[test]
    fn apply_updates_only_zero_frontier_rows_for_matching_article() {
        let archive = tempfile::tempdir().unwrap();
        let (_database_dir, database_path) = database();
        Connection::open(&database_path)
            .unwrap()
            .execute(
                "UPDATE frontier SET latest_fetch_time = 9876 WHERE url_id = 2",
                [],
            )
            .unwrap();
        write_page(
            archive.path(),
            "story.json",
            &page("https://example.com/story?page=2", Some(1234), vec![]),
        );

        let report = backfill_archive_timestamps(archive.path(), &database_path, true).unwrap();

        assert_eq!(report.frontier_rows_to_update, 1);
        assert_eq!(report.frontier_rows_updated, 1);
        assert_eq!(frontier_fetch_times(&database_path), vec![1234, 9876]);
    }

    #[test]
    fn uses_history_when_snapshots_do_not_have_a_timestamp() {
        let archive = tempfile::tempdir().unwrap();
        let (_database_dir, database_path) = database();
        write_page(
            archive.path(),
            "story.json",
            &page("https://example.com/story", None, vec![1200, 3400, 2300]),
        );

        let report = backfill_archive_timestamps(archive.path(), &database_path, true).unwrap();

        assert_eq!(report.frontier_rows_updated, 2);
        assert_eq!(frontier_fetch_times(&database_path), vec![3400, 3400]);
    }

    #[test]
    fn reports_archive_urls_without_matching_database_article() {
        let archive = tempfile::tempdir().unwrap();
        let (_database_dir, database_path) = database();
        write_page(
            archive.path(),
            "unknown.json",
            &page("https://example.com/not-in-database", Some(1234), vec![]),
        );

        let report = backfill_archive_timestamps(archive.path(), &database_path, true).unwrap();

        assert_eq!(report.matched_articles, 0);
        assert_eq!(
            report.unmatched_urls,
            vec!["https://example.com/not-in-database"]
        );
        assert_eq!(frontier_fetch_times(&database_path), vec![0, 0]);
    }

    #[test]
    fn malformed_archive_is_reported_and_other_files_are_processed() {
        let archive = tempfile::tempdir().unwrap();
        let (_database_dir, database_path) = database();
        write_page(
            archive.path(),
            "story.json",
            &page("https://example.com/story", Some(1234), vec![]),
        );
        fs::write(archive.path().join("invalid.json"), b"{ not json").unwrap();

        let report = backfill_archive_timestamps(archive.path(), &database_path, true).unwrap();

        assert_eq!(report.files_scanned, 2);
        assert_eq!(report.skipped_files.len(), 1);
        assert_eq!(
            report.skipped_files[0].path,
            archive.path().join("invalid.json").display().to_string()
        );
        assert!(
            report.skipped_files[0]
                .reason
                .contains("key must be a string")
        );
        assert_eq!(report.frontier_rows_updated, 2);
        assert_eq!(frontier_fetch_times(&database_path), vec![1234, 1234]);
    }

    #[test]
    fn reports_missing_archive_timestamp_and_processes_other_files() {
        let archive = tempfile::tempdir().unwrap();
        let (_database_dir, database_path) = database();
        write_page(
            archive.path(),
            "story.json",
            &page("https://example.com/story", None, vec![]),
        );
        write_page(
            archive.path(),
            "valid.json",
            &page("https://example.com/story", Some(8765), vec![]),
        );

        let report = backfill_archive_timestamps(archive.path(), &database_path, true).unwrap();

        assert_eq!(report.skipped_files.len(), 1);
        assert!(
            report.skipped_files[0]
                .reason
                .contains("no positive fetch timestamp")
        );
        assert_eq!(report.frontier_rows_updated, 2);
        assert_eq!(frontier_fetch_times(&database_path), vec![8765, 8765]);
    }

    #[test]
    fn accepts_nested_json_archives_and_ignores_other_files() {
        let archive = tempfile::tempdir().unwrap();
        let nested = archive.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("readme.txt"), "ignored").unwrap();
        let (_database_dir, database_path) = database();
        write_page(
            &nested,
            "story.json",
            &page("https://example.com/story", Some(1234), vec![]),
        );

        let report = backfill_archive_timestamps(archive.path(), &database_path, false).unwrap();

        assert_eq!(report.files_scanned, 1);
        assert_eq!(report.matched_articles, 1);
    }

    #[test]
    fn current_timestamp_takes_precedence_over_history() {
        let archived = page("https://example.com/story", Some(4500), vec![3400]);
        assert_eq!(latest_fetch_time(&archived).unwrap(), 4500);
    }

    #[test]
    fn uses_newest_historical_snapshot_when_current_metadata_is_missing() {
        let mut archived = page("https://example.com/story", None, vec![1200]);
        archived.historical_snapshots = vec![
            HistoricalSnapshot {
                content_markdown: Vec::new(),
                links: HashSet::new(),
                metadata: Some(metadata(1500)),
            },
            HistoricalSnapshot {
                content_markdown: Vec::new(),
                links: HashSet::new(),
                metadata: Some(metadata(2500)),
            },
        ]
        .into();
        assert_eq!(latest_fetch_time(&archived).unwrap(), 2500);
    }

    #[test]
    fn rejects_timestamps_outside_sqlite_integer_range() {
        let archived = page(
            "https://example.com/story",
            Some(i64::MAX as u64 + 1),
            vec![],
        );
        assert!(latest_fetch_time(&archived).is_err());
    }

    #[test]
    fn missing_database_is_rejected_without_creating_it() {
        let archive = tempfile::tempdir().unwrap();
        let database_path = archive.path().join("missing.db");
        assert!(backfill_archive_timestamps(archive.path(), &database_path, true).is_err());
        assert!(!database_path.exists());
    }

    #[test]
    fn groups_skipped_files_by_reason_without_json_location_noise() {
        let skipped = [
            SkippedArchiveFile {
                path: "one.json".to_owned(),
                reason: normalize_error_reason(&anyhow::anyhow!(
                    "missing field `task` at line 3502 column 1"
                )),
            },
            SkippedArchiveFile {
                path: "two.json".to_owned(),
                reason: normalize_error_reason(&anyhow::anyhow!(
                    "missing field `task` at line 17001 column 3"
                )),
            },
        ];

        let summaries = summarize_skipped_archive_files(&skipped);

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].count, 2);
        assert_eq!(summaries[0].reason, "missing field `task`");
        assert_eq!(summaries[0].example_path, "one.json");
    }
}
