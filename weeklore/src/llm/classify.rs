use serde::Deserialize;

use crate::llm::{LlmClient, prompt::fill_prompt, validate::validate_json};

#[derive(Clone, Debug, Deserialize)]
pub struct Category {
    pub category: String,
    #[serde(default)]
    pub additional_categories: Vec<String>,
    #[serde(default)]
    pub subcategories: Vec<String>,
}

impl LlmClient {
    pub async fn classify(&self, template: &str, summary: &str) -> anyhow::Result<Category> {
        let prompt = fill_prompt(template, &[("summary", summary)]);

        let result: Category = self
            .generate_with_retry(&prompt, validate_json::<Category>)
            .await?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn just_category() {
        let json_str = r#"{"category":"AI"}"#;
        let r: Result<Category, _> = serde_json::from_str(json_str);
        assert!(r.is_ok());
        let r = r.unwrap();
        assert_eq!(r.category, "AI");
        assert!(r.additional_categories.is_empty());
        assert!(r.subcategories.is_empty());
    }

    #[test]
    fn category_additionals() {
        let json_str = r#"{"category":"AI","additional_categories":["Programming","Business"]}"#;
        let r: Result<Category, _> = serde_json::from_str(json_str);
        assert!(r.is_ok());
        let r = r.unwrap();
        assert_eq!(r.category, "AI");
        assert_eq!(r.additional_categories, vec!["Programming", "Business"]);
        assert!(r.subcategories.is_empty());
    }

    #[test]
    fn category_and_subcategory() {
        let json_str = r#"{"category":"AI","subcategories":["Machine Learning","NLP"]}"#;
        let r: Result<Category, _> = serde_json::from_str(json_str);
        assert!(r.is_ok());
        let r = r.unwrap();
        assert_eq!(r.category, "AI");
        assert!(r.additional_categories.is_empty());
        assert_eq!(r.subcategories, vec!["Machine Learning", "NLP"]);
    }

    #[test]
    fn full_category() {
        let json_str = r#"{"category":"AI","additional_categories":["Programming"],"subcategories":["Computer Vision","Deep Learning"]}"#;
        let r: Result<Category, _> = serde_json::from_str(json_str);
        assert!(r.is_ok());
        let r = r.unwrap();
        assert_eq!(r.category, "AI");
        assert_eq!(r.additional_categories, vec!["Programming"]);
        assert_eq!(r.subcategories, vec!["Computer Vision", "Deep Learning"]);
    }
}
