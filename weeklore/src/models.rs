use anyhow::Context;
use common::markdown::html_to_markdown;

use crate::llm::Category;
use crate::{chunk::chunk_text, llm::LlmClient};

// models.rs
#[derive(Debug, Clone)]
pub struct Article {
    url: String,
    html: String,
}

#[derive(Debug, Clone)]
pub struct ExtractedArticle {
    url: String,
    text: String,
}

#[derive(Debug, Clone)]
pub struct ProcessedArticle {
    url: String,
    title: String,
    summary: Vec<String>,
    category: Category,
}

impl Article {
    pub async fn fetch(url: &str) -> anyhow::Result<Self> {
        let html = crate::fetch::fetch_url(url).await.context("fetching")?;

        Ok(Self {
            url: url.to_string(),
            html: html.to_string(),
        })
    }

    pub fn extract(self) -> ExtractedArticle {
        ExtractedArticle {
            url: self.url.clone(),
            text: html_to_markdown(&self.html, &self.url),
        }
    }
}

impl ExtractedArticle {
    pub async fn process(
        self,
        llm: &LlmClient,
        chunk_prompt: &str,
        reduce_prompt: &str,
        classify_prompt: &str,
    ) -> anyhow::Result<ProcessedArticle> {
        let chunks = chunk_text(&self.text, 3000);

        println!("process url: {:?}", self.url);

        // Step 1: summarise chunks
        let mut chunk_summaries = Vec::new();

        for chunk in chunks {
            let bullets = llm.summarise(chunk_prompt, &chunk).await?;
            chunk_summaries.extend(bullets);
        }

        // Step 2: reduce to page summary
        let combined = chunk_summaries.join("\n- ");

        let final_summary = llm.summarise(reduce_prompt, &combined).await?;

        // Step 3: classify
        let category = llm
            .classify(classify_prompt, &final_summary.join("\n"))
            .await?;

        Ok(ProcessedArticle {
            url: self.url.to_string(),
            title: self.url.to_string(), // upgrade later with title extraction
            summary: final_summary,
            category,
        })
    }
}

impl ProcessedArticle {
    pub fn category(&self) -> &str {
        self.category.category.as_str()
    }

    pub fn summary_with_url(&self) -> String {
        format!("- {}\n{}\n", self.url, self.summary.join("\n"))
    }

    pub fn bullet(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!("### {}\n", self.title));
        output.push_str(&format!("Source: {}\n", self.url));

        if !self.category.subcategories.is_empty() {
            output.push_str(&format!(
                "Sub-Categories (to {}): {:?}\n",
                self.category.category, self.category.subcategories
            ));
        }

        if !self.category.additional_categories.is_empty() {
            output.push_str(&format!(
                "Additional Categories: {:?}\n",
                self.category.additional_categories
            ));
        }

        for bullet in &self.summary {
            output.push_str(&format!("- {}\n", bullet));
        }

        output
    }
}
