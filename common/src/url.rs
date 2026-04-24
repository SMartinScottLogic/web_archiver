use std::collections::HashSet;

use url::{Url, form_urlencoded};

// =========================
// Shared helpers
// =========================

#[derive(Debug, PartialEq)]
pub enum Page<T> {
    Number(T),
    Text(String),
    None,
}

fn is_default_port(scheme: &str, port: u16) -> bool {
    match scheme {
        "http" => port == 80,
        "https" => port == 443,
        _ => false,
    }
}

fn pagination_params() -> &'static [&'static str] {
    &[
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
    ]
}

fn is_ignored_param(key: &str) -> bool {
    // Tracking / analytics
    key.starts_with("utm_")
        || matches!(
            key,
            // Ad / click tracking
            |"gclid"| "fbclid"
            | "dclid"
            | "msclkid"
            | "igshid"
        // Email / marketing systems
            | "mc_cid"
            | "mc_eid"
        // Generic referrer / source tracking
            | "ref"
            | "ref_src"
            | "source"
            | "src"
        // Social / platform noise
            | "feature"
            | "si"
        // Others
            | "_ga"
            | "session"
            | "sessionid"
            | "phpsessid"
            | "sid"
        )
}

fn encode_sorted_query(pairs: &[(String, String)]) -> String {
    pairs
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
        .finish()
}

fn filter_query_pairs<F>(url: &Url, mut filter: F, dedupe: bool) -> Vec<(String, String)>
where
    F: FnMut(&str) -> bool,
{
    let mut seen = HashSet::new();

    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .into_owned()
        .filter(|(k, _)| filter(k))
        .filter(|(k, v)| {
            if !dedupe {
                return true;
            }
            seen.insert((k.clone(), v.clone()))
        })
        .collect();

    pairs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    pairs
}

fn rebuild_url(parsed: &Url) -> String {
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

// =========================
// Public API
// =========================

pub fn resolve_relative_link(base: &str, link: &str) -> Option<String> {
    let base_url = Url::parse(base).ok()?;
    base_url.join(link).ok().map(|u| u.to_string())
}

pub fn canonicalize_url(input: &str) -> Option<String> {
    let mut url = Url::parse(input).ok()?;

    url.set_fragment(None);

    let scheme = url.scheme().to_lowercase();
    let host = url.host_str()?.to_lowercase();

    let mut normalized = format!("{}://{}", scheme, host);

    if let Some(port) = url.port()
        && !is_default_port(&scheme, port)
    {
        normalized.push_str(&format!(":{}", port));
    }

    normalized.push_str(url.path());

    let pairs = filter_query_pairs(
        &url,
        |k| !is_ignored_param(k),
        true, // dedupe
    );

    let encoded = encode_sorted_query(&pairs);

    if !encoded.is_empty() {
        normalized.push('?');
        normalized.push_str(&encoded);
    }

    Some(normalized)
}

pub fn remove_pagination_params(input: &str) -> String {
    let mut parsed = match Url::parse(input) {
        Ok(u) => u,
        Err(_) => return input.to_string(),
    };

    let pairs = filter_query_pairs(&parsed, |k| !pagination_params().contains(&k), false);

    let encoded = encode_sorted_query(&pairs);

    if encoded.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.set_query(Some(&encoded));
    }

    rebuild_url(&parsed)
}

pub fn normalize_url_for_merge(url_str: &str) -> Option<String> {
    let mut parsed = Url::parse(url_str).ok()?;

    let pairs = filter_query_pairs(&parsed, |k| !pagination_params().contains(&k), false);

    let encoded = encode_sorted_query(&pairs);

    if encoded.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.set_query(Some(&encoded));
    }

    Some(parsed.to_string())
}

pub fn extract_page(url_str: &str) -> Page<u32> {
    let parsed = match Url::parse(url_str).ok() {
        Some(p) => p,
        None => return Page::None,
    };

    for (key, value) in parsed.query_pairs() {
        if pagination_params().contains(&key.as_ref()) {
            return match value.parse::<u32>() {
                Ok(n) => Page::Number(n),
                Err(_err) => Page::Text(value.to_string()),
            };
        }
    }

    Page::None
}

pub fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

pub fn hash_url(url: &str) -> String {
    blake3::hash(url.as_bytes()).to_hex().to_string()
}

pub fn url_to_filename(url: &str) -> String {
    let url = url.strip_prefix("https://").unwrap_or(url);
    let url = url.strip_prefix("http://").unwrap_or(url);

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

    let filename = filename.trim_matches('-');

    if filename.is_empty() || filename.ends_with(['\\', '/']) {
        format!("{}{}", filename, "index")
    } else {
        filename.to_string()
    }
}

pub fn extract_domain(input: &str) -> Option<String> {
    Url::parse(input).ok()?.host_str().map(|s| s.to_string())
}

pub fn sanitize_segment(input: &str) -> String {
    let mut out = String::with_capacity(50);
    let mut last_was_underscore = false;

    for c in input.chars() {
        if out.len() == 50 {
            break;
        }

        let c = if c == ' ' || c == '.' { '_' } else { c };

        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            if c == '_' {
                if out.is_empty() || last_was_underscore {
                    continue;
                }
                out.push('_');
                last_was_underscore = true;
            } else {
                out.push(c);
                last_was_underscore = false;
            }
        }
    }

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
        assert_eq!(url_to_filename("https://example.com/"), "example.com/index");
        assert_eq!(url_to_filename(""), "index");
    }

    #[test]
    fn test_normalize_url_removes_page_param() {
        let url = "http://example.com/article?page=2";
        assert_eq!(
            normalize_url_for_merge(url),
            Some("http://example.com/article".to_string())
        );
    }

    #[test]
    fn test_normalize_url_preserves_other_params() {
        let url = "http://example.com/forum?sort=date&page=3";
        let result = normalize_url_for_merge(url).unwrap();
        assert!(result.contains("sort=date"));
        assert!(!result.contains("page"));
    }

    #[test]
    fn test_normalize_url_removes_multiple_pagination_params() {
        let url = "http://example.com/search?q=test&page=2&offset=10&sort=recent";
        let result = normalize_url_for_merge(url).unwrap();
        assert!(result.contains("q=test"));
        assert!(result.contains("sort=recent"));
        assert!(!result.contains("page"));
        assert!(!result.contains("offset"));
    }

    #[test]
    fn test_normalize_url_with_p_param() {
        let url = "http://example.com/forum?id=123&p=5";
        let result = normalize_url_for_merge(url).unwrap();
        assert!(result.contains("id=123"));
        assert!(!result.contains("p="));
    }

    #[test]
    fn test_extract_page_number_from_page_param() {
        let url = "http://example.com/article?page=3";
        assert_eq!(extract_page(url), Page::Number(3));
    }

    #[test]
    fn test_extract_page_number_from_p_param() {
        let url = "http://example.com/forum?id=123&p=7";
        assert_eq!(extract_page(url), Page::Number(7));
    }

    #[test]
    fn test_extract_page_number_not_found() {
        let url = "http://example.com/article?id=123&author=john";
        assert_eq!(extract_page(url), Page::None);
    }

    #[test]
    fn test_extract_page_number_non_numeric() {
        let url = "http://example.com/article?page=latest";
        assert_eq!(extract_page(url), Page::Text("latest".to_string()));
    }

    #[test]
    fn test_normalize_invalid_url() {
        let url = "not a valid url";
        assert_eq!(normalize_url_for_merge(url), None);
    }
}
