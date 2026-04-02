use std::collections::HashSet;

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

    // Common params to ignore
    fn is_ignored_param(key: &str) -> bool {
        matches!(
            key,
            // UTM tracking
            "utm_source"
            | "utm_medium"
            | "utm_campaign"
            | "utm_term"
            | "utm_content"
            // Analytics
            | "gclid"
            | "fbclid"
            | "_ga"
            // Session / tracking
            | "session"
            | "sessionid"
            | "phpsessid"
            | "sid"
            | "ref"
        )
    }
    let mut seen = HashSet::new();

    let mut p = url
        .query_pairs()
        .filter(|(k, _)| !is_ignored_param(k))
        .filter(|(k, v)| {
            // dedupe based on (key, value)
            let pair = (k.to_string(), v.to_string());
            seen.insert(pair)
        })
        .collect::<Vec<_>>();

    // Sort for stable canonical form
    p.sort_by(|(k1, v1), (k2, v2)| k1.cmp(k2).then(v1.cmp(v2)));

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

/// Remove pagination-related query parameters from a URL.
/// Returns the URL with pagination parameters stripped, or original if invalid.
pub fn remove_pagination_params(input: &str) -> String {
    let mut parsed = match Url::parse(input) {
        Ok(url) => url,
        Err(_) => return input.to_string(),
    };

    let pagination_params = [
        "page",
        "p",
        "offset",
        "start",
        "begin",
        "begin_idx",
        "idx",
        "from",
        "_start",
        "_skip",
        "limit",
        "pn",
    ];

    let filtered: Vec<(String, String)> = parsed
        .query_pairs()
        .into_owned()
        .filter(|(k, _)| !pagination_params.contains(&k.as_str()))
        .collect();

    let mut sorted = filtered;
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let encoded = sorted
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

    if encoded.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.set_query(Some(&encoded));
    }

    // Reconstruct URL while preserving unslashed host for root URLs
    let mut output = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));

    if let Some(port) = parsed.port()
        && !is_default_port(parsed.scheme(), port)
    {
        output.push(':');
        output.push_str(&port.to_string());
    }

    let path = parsed.path();
    if !path.is_empty() && path != "/" {
        output.push_str(path);
    }

    if let Some(q) = parsed.query() {
        output.push('?');
        output.push_str(q);
    }

    output
}

fn is_default_port(scheme: &str, port: u16) -> bool {
    match scheme {
        "http" => port == 80,
        "https" => port == 443,
        _ => false,
    }
}

pub fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Generate a stable hash for a URL.
/// Used for fast deduplication and filenames.
pub fn hash_url(url: &str) -> String {
    blake3::hash(url.as_bytes()).to_hex().to_string()
}

/// Convert a URL to a filesystem-safe filename that approximates the URL.
/// Removes protocol, replaces unsafe characters with hyphens, limits length.
pub fn url_to_filename(url: &str) -> String {
    // Remove protocol
    let url = url.strip_prefix("https://").unwrap_or(url);
    let url = url.strip_prefix("http://").unwrap_or(url);

    // Replace unsafe characters with hyphens
    let mut filename = String::with_capacity(200);
    for c in url.chars() {
        if filename.len() >= 200 {
            break;
        }
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
            filename.push(c);
        } else if c == '/' || c == '\\' {
            filename.push(std::path::MAIN_SEPARATOR);
        } else {
            filename.push('-');
        }
    }

    // Remove leading/trailing hyphens
    let filename = filename.trim_matches('-');

    // Ensure not empty
    if filename.is_empty() {
        "index".to_string()
    } else {
        filename.to_string()
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

pub fn sanitize(input: &str) -> String {
    let mut out = String::with_capacity(50);

    let mut last_was_underscore = false;

    for c in input.chars() {
        if out.len() == 50 {
            break;
        }

        let c = if c == ' ' || c == '.' { '_' } else { c };

        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            if c == '_' {
                // Skip leading underscore or repeated ones
                if out.is_empty() || last_was_underscore {
                    continue;
                }

                // Only push if there's room AND it's not going to be trailing
                if out.len() < 50 {
                    out.push('_');
                    last_was_underscore = true;
                }
            } else {
                out.push(c);
                last_was_underscore = false;
            }
        }
    }

    // Remove trailing underscore if present
    if out.ends_with('_') {
        out.pop();
    }

    if out.is_empty() { "_".to_string() } else { out }
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
    fn test_url_to_filename() {
        assert_eq!(
            url_to_filename("https://example.com/page"),
            "example.com/page"
        );
        assert_eq!(
            url_to_filename("http://test.com/path/to/file.html"),
            "test.com/path/to/file.html"
        );
        assert_eq!(
            url_to_filename("https://example.com/path?query=value#fragment"),
            "example.com/path-query-value-fragment"
        );
        assert_eq!(url_to_filename("https://example.com/"), "example.com/");
        assert_eq!(url_to_filename(""), "index");
    }
}
