use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use url::{Url, form_urlencoded};

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
        && !is_default_port(&scheme, port)
    {
        normalized.push_str(&format!(":{}", port));
    }

    normalized.push_str(url.path());
    let mut p = url.query_pairs().collect::<Vec<_>>();
    p.sort_by_cached_key(|(k, _v)| k.to_string());

    let encoded = p
        .iter()
        .fold(
            form_urlencoded::Serializer::new(String::new()),
            |mut acc, (k, v)| {
                if v.is_empty() {
                    acc.append_key_only(k);
                } else {
                    acc.append_pair(k, v);
                }
                acc
            },
        )
        .finish();
    url.set_query(Some(&encoded));

    if !encoded.is_empty() {
        normalized.push('?');
        normalized.push_str(&encoded);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_relative_link() {
        let base = "https://example.com/page";
        let link = "/about";
        assert_eq!(
            resolve_relative_link(base, link),
            Some("https://example.com/about".to_string())
        );
    }

    #[test]
    fn test_canonicalize_url() {
        let url = "HTTP://Example.com:80/foo?bar#frag";
        assert_eq!(
            canonicalize_url(url),
            Some("http://example.com/foo?bar".to_string())
        );
    }

    #[test]
    fn test_canonicalize_url_http_custom_port() {
        let url = "HTTP://Example.com:8080/foo?bar#frag";
        assert_eq!(
            canonicalize_url(url),
            Some("http://example.com:8080/foo?bar".to_string())
        );
    }

    #[test]
    fn test_canonicalize_url_https_custom_port() {
        let url = "HTTPS://Example.com:8080/foo?bar#frag";
        assert_eq!(
            canonicalize_url(url),
            Some("https://example.com:8080/foo?bar".to_string())
        );
    }

    #[test]
    fn test_canonicalize_url_query_order() {
        let url = "http://example.com/?b[]=2&a=1&b[]=c";
        assert_eq!(
            canonicalize_url(url),
            Some("http://example.com/?a=1&b%5B%5D=2&b%5B%5D=c".to_string())
        );
    }

    #[test]
    fn test_extract_domain() {
        let url = "https://news.ycombinator.com/item?id=1";
        assert_eq!(
            extract_domain(url),
            Some("news.ycombinator.com".to_string())
        );
    }

    #[test]
    fn test_hash_url_stable() {
        let url = "https://foo.com";
        assert_eq!(hash_url(url), hash_url(url));
    }

    #[test]
    fn test_is_http_url() {
        assert!(is_http_url("http://foo.com"));
        assert!(is_http_url("https://foo.com"));
        assert!(!is_http_url("ftp://foo.com"));
    }

    #[test]
    fn test_is_default_port() {
        assert!(is_default_port("http", 80));
        assert!(is_default_port("https", 443));
        assert!(!is_default_port("http", 8080));
    }
}
