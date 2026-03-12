use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use url::Url;

/// Resolve a possibly relative link against a base URL.
///
/// Example:
/// base: https://example.com/page
/// link: /about
/// result: https://example.com/about
pub fn resolve_relative_link(base: &str, link: &str) -> Option<String> {
    let base_url = Url::parse(base).ok()?;

    match base_url.join(link) {
        Ok(resolved) => Some(resolved.to_string()),
        Err(_) => None,
    }
}

/// Basic canonicalization for Week 1.
///
/// Rules:
/// - remove fragment (#section)
/// - normalize scheme and host casing
/// - remove default ports
pub fn canonicalize_url(input: &str) -> Option<String> {
    let mut url = Url::parse(input).ok()?;

    // Remove fragment
    url.set_fragment(None);

    // Normalize scheme + host to lowercase
    let scheme = url.scheme().to_lowercase();
    let host = url.host_str()?.to_lowercase();

    let mut normalized = format!("{}://{}", scheme, host);

    if let Some(port) = url.port()
        && !is_default_port(&scheme, port) {
            normalized.push_str(&format!(":{}", port));
        }

    normalized.push_str(url.path());

    if let Some(query) = url.query() {
        normalized.push('?');
        normalized.push_str(query);
    }

    Some(normalized)
}

fn is_default_port(scheme: &str, port: u16) -> bool {
    match scheme {
        "http" => port == 80,
        "https" => port == 443,
        _ => false,
    }
}

/// Extract domain from URL.
///
/// Example:
/// https://news.ycombinator.com/item?id=1
/// -> news.ycombinator.com
pub fn extract_domain(input: &str) -> Option<String> {
    let url = Url::parse(input).ok()?;
    url.host_str().map(|s| s.to_string())
}

/// Generate a stable hash for a URL.
/// Used for fast deduplication and filenames.
pub fn hash_url(url: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    hasher.finish()
}

pub fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}
