use anyhow::Context;
use common::markdown::html_to_markdown;

use crate::chunk::chunk_text;
use crate::llm::Category;
use crate::llm::classify::LlmClassify;
use crate::llm::client::LlmSummarise;

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
    pub async fn process<L>(
        self,
        llm: &L,
        chunk_prompt: &str,
        reduce_prompt: &str,
        classify_prompt: &str,
    ) -> anyhow::Result<ProcessedArticle>
    where
        L: LlmClassify + LlmSummarise,
    {
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

#[cfg(test)]
mod mocks {
    use super::*;
    use crate::llm::{Category, client::LlmSummarise};

    pub struct MockLlmClient;

    impl LlmSummarise for MockLlmClient {
        async fn summarise(&self, _prompt: &str, input: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![format!("summary of {}", input)])
        }
    }

    impl LlmClassify for MockLlmClient {
        async fn classify(&self, _prompt: &str, _input: &str) -> anyhow::Result<Category> {
            Ok(Category {
                category: "test".to_string(),
                subcategories: vec!["sub1".to_string()],
                additional_categories: vec!["extra".to_string()],
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_article_extract() {
        let article = Article {
            url: "http://example.com".to_string(),
            html: "<p>Hello</p>".to_string(),
        };

        let extracted = article.extract();

        assert_eq!(extracted.url, "http://example.com");
        assert!(extracted.text.contains("Hello"));
    }

    #[tokio::test]
    async fn test_process_article_basic() {
        use mocks::MockLlmClient;

        let extracted = ExtractedArticle {
            url: "http://example.com".to_string(),
            text: "This is a test article.".to_string(),
        };

        let llm = MockLlmClient;

        let result = extracted
            .process(&llm, "chunk", "reduce", "classify")
            .await
            .unwrap();

        assert_eq!(result.url, "http://example.com");
        assert_eq!(result.category(), "test");
        assert!(!result.summary.is_empty());
    }

    #[tokio::test]
    async fn test_process_multiple_chunks() {
        use mocks::MockLlmClient;

        let long_text = "a".repeat(10_000); // force multiple chunks

        let extracted = ExtractedArticle {
            url: "http://example.com".to_string(),
            text: long_text,
        };

        let llm = MockLlmClient;

        let result = extracted
            .process(&llm, "chunk", "reduce", "classify")
            .await
            .unwrap();

        assert!(!result.summary.is_empty());
    }

    #[test]
    fn test_category_accessor() {
        let article = ProcessedArticle {
            url: "url".to_string(),
            title: "title".to_string(),
            summary: vec![],
            category: Category {
                category: "news".to_string(),
                subcategories: vec![],
                additional_categories: vec![],
            },
        };

        assert_eq!(article.category(), "news");
    }

    #[test]
    fn test_summary_with_url() {
        let article = ProcessedArticle {
            url: "http://example.com".to_string(),
            title: "title".to_string(),
            summary: vec!["point1".to_string(), "point2".to_string()],
            category: Category {
                category: "test".to_string(),
                subcategories: vec![],
                additional_categories: vec![],
            },
        };

        let output = article.summary_with_url();

        assert!(output.contains("http://example.com"));
        assert!(output.contains("point1"));
        assert!(output.contains("point2"));
    }

    #[test]
    fn test_bullet_formatting() {
        let article = ProcessedArticle {
            url: "http://example.com".to_string(),
            title: "Test Title".to_string(),
            summary: vec!["bullet1".to_string()],
            category: Category {
                category: "main".to_string(),
                subcategories: vec!["sub1".to_string()],
                additional_categories: vec!["extra1".to_string()],
            },
        };

        let output = article.bullet();

        assert!(output.contains("### Test Title"));
        assert!(output.contains("Source: http://example.com"));
        assert!(output.contains("sub1"));
        assert!(output.contains("extra1"));
        assert!(output.contains("- bullet1"));
    }

    #[test]
    fn test_bullet_empty_fields() {
        let article = ProcessedArticle {
            url: "url".to_string(),
            title: "title".to_string(),
            summary: vec![],
            category: Category {
                category: "main".to_string(),
                subcategories: vec![],
                additional_categories: vec![],
            },
        };

        let output = article.bullet();

        assert!(output.contains("### title"));
        assert!(!output.contains("Sub-Categories"));
        assert!(!output.contains("Additional Categories"));
    }
}
