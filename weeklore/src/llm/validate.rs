pub fn validate_bullets(output: &str, min: usize, max: usize) -> bool {
    let count = output
        .lines()
        .filter(|l| l.trim().starts_with(['-', '*']))
        .count();

    count >= min && count <= max
}

pub fn extract_json(output: &str) -> Option<String> {
    let start = output.find('{')?;
    let end = output.rfind('}')?;
    Some(output[start..=end].to_string())
}

pub fn validate_json<T: serde::de::DeserializeOwned>(output: &str) -> Option<T> {
    let json_str = extract_json(output)?;
    serde_json::from_str(&json_str).ok()
}
