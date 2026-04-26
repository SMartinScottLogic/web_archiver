use crate::page::PageReader;
use std::path::PathBuf;

use mockall::automock;

mod balanced_archiver;
mod full_path_archiver;
mod historical_archiver;

#[automock]
pub trait Archiver {
    fn for_path(archive_dir: PathBuf) -> Self;
    fn canonical_filename(&self, url_str: &str, datetime: i64) -> anyhow::Result<PathBuf>;
    fn generate_filename(&self, page: &dyn PageReader) -> anyhow::Result<PathBuf>;
    fn store_page(&self, page: &dyn PageReader) -> anyhow::Result<PathBuf>;
}
pub use historical_archiver::HistoricalArchiver as DefaultArchiver;
