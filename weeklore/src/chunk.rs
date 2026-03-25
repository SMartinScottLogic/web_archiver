// chunk.rs
pub fn chunk_text(text: &str, max_len: usize) -> Vec<String> {
    text.as_bytes()
        .chunks(max_len)
        .map(|c| String::from_utf8_lossy(c).to_string())
        .collect()
}
