// ---------------------------
// Markdown-aware chunking
// ---------------------------

pub struct Chunk {
    pub text: String,
}

pub fn chunk_markdown(text: &str, chunk_size: usize, overlap: usize) -> Vec<Chunk> {
    let blocks = split_markdown_blocks(text);

    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut current_tokens = 0;

    for block in blocks {
        let tokens = estimate_tokens(&block);

        // If adding this block exceeds chunk size → flush
        if current_tokens + tokens > chunk_size && !current.is_empty() {
            let chunk_text = current.join("\n\n");

            chunks.push(Chunk {
                text: chunk_text.clone(),
            });

            // Handle overlap
            let overlap_text = take_overlap(&chunk_text, overlap);
            current = vec![overlap_text];
            current_tokens = estimate_tokens(&current[0]);
        }

        current.push(block);
        current_tokens += tokens;
    }

    // अंतिम chunk
    if !current.is_empty() {
        chunks.push(Chunk {
            text: current.join("\n\n"),
        });
    }

    chunks
}

fn split_markdown_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = Vec::new();

    for line in text.lines() {
        // New section on headings
        if line.starts_with('#') && !current.is_empty() {
            blocks.push(current.join("\n"));
            current.clear();
        }

        // Paragraph break
        if line.trim().is_empty() && !current.is_empty() {
            blocks.push(current.join("\n"));
            current.clear();
            continue;
        }

        current.push(line.to_string());
    }

    if !current.is_empty() {
        blocks.push(current.join("\n"));
    }

    blocks
}

fn estimate_tokens(text: &str) -> usize {
    text.split_whitespace().count()
}

fn take_overlap(text: &str, overlap_tokens: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();

    let start = if words.len() > overlap_tokens {
        words.len() - overlap_tokens
    } else {
        0
    };

    words[start..].join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(
            13,
            estimate_tokens(
                "Estimate tokens simply returns the number of   words in a block of text"
            )
        );
    }

    #[test]
    fn test_split_markdown_blocks() {
        let text = "# Heading 1\n\nParagraph 1\n\n## Heading 2\n\nParagraph 2";
        let blocks = split_markdown_blocks(text);
        assert_eq!(4, blocks.len());
        assert_eq!("# Heading 1", blocks[0]);
        assert_eq!("Paragraph 1", blocks[1]);
        assert_eq!("## Heading 2", blocks[2]);
        assert_eq!("Paragraph 2", blocks[3]);
    }

    #[test]
    fn test_chunk_markdown() {
        let text = "This is a long text\n\nWith multiple\n\nParagraphs and\n\nHeadings";
        let chunks = chunk_markdown(text, 7, 2);
        assert_eq!(2, chunks.len());
        assert_eq!("This is a long text\n\nWith multiple", chunks[0].text);
        assert_eq!(
            "With multiple\n\nParagraphs and\n\nHeadings",
            chunks[1].text
        );
    }

    #[test]
    fn test_take_overlap() {
        let text = "This is a long text with overlap";
        let overlap = take_overlap(text, 3);
        assert_eq!("text with overlap", overlap);
    }
}
