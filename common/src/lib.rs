use mockall::predicate::*;
use mockall::*;
use std::path::PathBuf;

pub mod balanced_archiver;
pub mod compressed_string;
pub mod full_path_archiver;
pub mod historical;
pub mod markdown;
pub mod page;
pub mod reqwest_ext;
pub mod settings;
pub mod types;
pub mod url;

#[automock]
pub trait Archiver {
    fn generate_filename(&self, page: &types::ExtractedPage) -> anyhow::Result<PathBuf>;
    fn store_page(&self, page: &types::ExtractedPage) -> anyhow::Result<PathBuf>;
}
pub use balanced_archiver::BalancedArchiver as DefaultArchiver;
