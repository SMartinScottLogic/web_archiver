use url::Url;

/// Normalize a URL for merging multi-page snapshots.
///
/// Removes pagination-related query parameters like ?page=X, ?offset=Y, ?p=X, etc.
/// Preserves other meaningful parameters that identify different content.
///
/// Examples:
/// - "http://example.com/article?page=2" -> "http://example.com/article"
/// - "http://example.com/forum?p=1&sort=date" -> "http://example.com/forum?sort=date"
/// - "http://example.com/search?q=test&page=5" -> "http://example.com/search?q=test"
pub fn normalize_url_for_merge(url_str: &str) -> Option<String> {
    let mut parsed = Url::parse(url_str).ok()?;

    // List of pagination-related parameters to remove
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

    // Collect non-pagination parameters using into_owned to get String types
    let filtered_pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .into_owned()
        .filter(|(k, _): &(String, String)| !pagination_params.contains(&k.as_str()))
        .collect();

    // Sort for deterministic output
    let mut sorted_pairs = filtered_pairs.clone();
    sorted_pairs.sort_by(|a, b| a.0.cmp(&b.0));

    // Rebuild the query string with filtered parameters
    let mut query_string = String::new();
    for (i, (key, value)) in sorted_pairs.iter().enumerate() {
        if i > 0 {
            query_string.push('&');
        }
        query_string.push_str(key);
        query_string.push('=');
        query_string.push_str(value);
    }

    // Set the new query string (or clear it if empty)
    if query_string.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.set_query(Some(&query_string));
    }

    Some(parsed.to_string())
}

/// Extract the page number from a URL if present.
///
/// Looks for common pagination parameters and returns their numeric value.
/// Returns None if no page parameter found or if it's not numeric.
pub fn extract_page_number(url_str: &str) -> Option<u32> {
    let parsed = Url::parse(url_str).ok()?;

    let pagination_params = ["page", "p", "offset", "start", "begin", "idx", "pn"];

    let owned_pairs: Vec<(String, String)> = parsed.query_pairs().into_owned().collect();

    for param in &pagination_params {
        for (key, value) in &owned_pairs {
            if key == *param
                && let Ok(page_num) = value.as_str().parse::<u32>()
            {
                return Some(page_num);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(extract_page_number(url), Some(3));
    }

    #[test]
    fn test_extract_page_number_from_p_param() {
        let url = "http://example.com/forum?id=123&p=7";
        assert_eq!(extract_page_number(url), Some(7));
    }

    #[test]
    fn test_extract_page_number_not_found() {
        let url = "http://example.com/article?id=123&author=john";
        assert_eq!(extract_page_number(url), None);
    }

    #[test]
    fn test_extract_page_number_non_numeric() {
        let url = "http://example.com/article?page=latest";
        assert_eq!(extract_page_number(url), None);
    }

    #[test]
    fn test_normalize_invalid_url() {
        let url = "not a valid url";
        assert_eq!(normalize_url_for_merge(url), None);
    }
}
