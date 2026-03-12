pub mod markdown;
/// Utilities for the crawler.
pub mod url;

pub use markdown::html_to_markdown;
/// Re-export commonly used functions for convenience.
pub use url::{canonicalize_url, extract_domain, hash_url, resolve_relative_link};
