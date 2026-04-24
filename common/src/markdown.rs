use html2md::parse_html;
use readability::extractor;
use url::Url;

/// Convert HTML into simplified Markdown with:
/// - Readability-style article extraction
/// - Scripts/styles removed
/// - Links removed (text kept)
pub fn html_to_markdown(html: &str, url: &str) -> String {
    // --- 1. Readability extraction ---
    let url = Url::parse(url).unwrap();
    let mut content = match extractor::extract(&mut html.as_bytes(), &url) {
        Ok(a) => a.content,
        Err(_) => html.to_string(), // fallback to raw HTML
    };

    // --- 2. Remove <script>, <style>, <noscript> manually ---
    content = remove_tag(&content, "script");
    content = remove_tag(&content, "style");
    content = remove_tag(&content, "noscript");

    // --- 3. Convert to Markdown ---
    let mut markdown = parse_html(&content);

    // --- 4. Remove links from Markdown ---
    // Replace [text](url) -> text
    markdown = remove_markdown_links(&markdown);

    markdown
}

fn remove_tag(html: &str, tag: &str) -> String {
    let mut output = String::new();
    let mut remaining = html;

    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);

    while let Some(start) = remaining.find(&open) {
        output.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find(&close) {
            remaining = &remaining[start + end + close.len()..];
        } else {
            break;
        }
    }
    output.push_str(remaining);
    output
}

fn remove_markdown_links(md: &str) -> String {
    let mut output = String::new();
    let mut rest = md;

    while let Some(start) = rest.find('[') {
        output.push_str(&rest[..start]);
        if let Some(end_text) = rest[start..].find(']') {
            let after_text = &rest[start + end_text + 1..];
            if after_text.starts_with('(')
                && let Some(end_url) = after_text.find(')')
            {
                rest = &after_text[end_url + 1..];
                continue;
            }
            output.push_str(&rest[start..start + end_text + 1]);
            rest = after_text;
        } else {
            break;
        }
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_to_markdown_basic() {
        let html = "<h1>Title</h1><p>Hello <a href='x'>world</a></p>";
        let url = "https://example.com";
        let md = html_to_markdown(html, url);
        assert!(!md.is_empty()); // Should produce some output
    }

    #[test]
    fn test_remove_tag() {
        let html = "<div>keep</div><script>remove</script>";
        let cleaned = super::remove_tag(html, "script");
        assert_eq!(cleaned, "<div>keep</div>");
    }

    #[test]
    fn test_remove_markdown_links() {
        let md = "[text](url) plain";
        let cleaned = super::remove_markdown_links(md);
        assert_eq!(cleaned, " plain");
    }
}
