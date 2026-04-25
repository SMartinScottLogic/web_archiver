use mockall::predicate::*;
use mockall::*;
use std::path::PathBuf;

pub mod balanced_archiver;
pub mod compressed_string;
pub mod full_path_archiver;
pub mod historical;
pub mod historical_archiver;
pub mod markdown;
pub mod page;
pub mod reqwest_ext;
pub mod settings;
pub mod types;
pub mod url;
mod json_ld;

#[automock]
pub trait Archiver {
    fn for_path(archive_dir: PathBuf) -> Self;
    fn canonical_filename(&self, url_str: &str, datetime: i64) -> anyhow::Result<PathBuf>;
    fn generate_filename(&self, page: &dyn page::PageReader) -> anyhow::Result<PathBuf>;
    fn store_page(&self, page: &dyn page::PageReader) -> anyhow::Result<PathBuf>;
}
pub use historical_archiver::HistoricalArchiver as DefaultArchiver;
pub use json_ld::JsonLd;
pub use json_ld::parse as parse_jsonld;
