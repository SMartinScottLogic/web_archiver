// pipeline.rs
use crate::llm::LlmClient;
use crate::models::{Article, ProcessedArticle};
use std::collections::HashMap;

pub async fn process_url(
    llm: &LlmClient,
    url: &str,
    chunk_prompt: &str,
    reduce_prompt: &str,
    classify_prompt: &str,
) -> anyhow::Result<ProcessedArticle> {
    let article = Article::fetch(url).await?;

    let extracted = article.extract();

    let processed = extracted
        .process(llm, chunk_prompt, reduce_prompt, classify_prompt)
        .await?;

    Ok(processed)
}

pub async fn build_report(
    llm: &LlmClient,
    articles: Vec<ProcessedArticle>,
    exec_prompt: &str,
) -> anyhow::Result<String> {
    let mut grouped: HashMap<String, Vec<&ProcessedArticle>> = HashMap::new();

    for article in &articles {
        grouped
            .entry(article.category().to_string())
            .or_default()
            .push(article);
    }

    // Build input for LLM
    let mut grouped_text = String::new();

    for (cat, items) in &grouped {
        grouped_text.push_str(&format!("## {}\n", cat));

        for a in items {
            grouped_text.push_str(&a.summary_with_url());
        }
    }

    let exec = llm
        .generate(
            exec_prompt
                .replace("{grouped_summaries}", &grouped_text)
                .as_str(),
        )
        .await?;

    // Combine into markdown
    let mut output = String::new();

    output.push_str("# Weekly Reading Report\n\n");
    output.push_str(&exec);
    output.push_str("\n\n");

    for (cat, items) in grouped {
        output.push_str(&format!("## {}\n\n", cat));

        for a in items {
            output.push_str(&a.bullet());
            output.push('\n');
        }
    }

    Ok(output)
}
