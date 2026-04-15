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

pub trait LlmClassify {
    async fn classify(&self, template: &str, summary: &str) -> anyhow::Result<Category>;
}

impl LlmClassify for LlmClient {
    async fn classify(&self, template: &str, summary: &str) -> anyhow::Result<Category> {
        let prompt = fill_prompt(template, &[("summary", summary)]);

        let result: Category = self
            .generate_with_retry(&prompt, validate_json::<Category>)
            .await?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use crate::llm::tests::SequenceResponder;

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

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn classify_success_first_try() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                r#"{"response":"{\"category\":\"sports\",\"additional_categories\":[],\"subcategories\":[]}" }"#,
            ),
        )
        .mount(&server)
        .await;

        let client = LlmClient::new(&server.uri());

        let result = client.classify("template", "some summary").await.unwrap();

        assert_eq!(result.category, "sports");
        assert!(result.additional_categories.is_empty());
        assert!(result.subcategories.is_empty());

        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn classify_retries_until_valid_json() {
        let server = MockServer::start().await;

        let responses = vec![
    // First: invalid JSON → should be rejected by validate_json
        ResponseTemplate::new(200)
                .set_body_string(r#"{"response":"not-json"}"#),
    // Second: valid Category JSON
        ResponseTemplate::new(200).set_body_string(
                r#"{"response":"{\"category\":\"sports\",\"additional_categories\":[\"news\"],\"subcategories\":[\"football\"]}" }"#,)
    ];

        let responder = SequenceResponder::new(responses);

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(responder)
            .mount(&server)
            .await;

        let client = LlmClient::new(&server.uri());

        let result = client.classify("template", "summary text").await.unwrap();

        assert_eq!(result.category, "sports");
        assert_eq!(result.additional_categories, vec!["news"]);
        assert_eq!(result.subcategories, vec!["football"]);

        let calls = server.received_requests().await.unwrap().len();
        assert_eq!(calls, 2);
    }

    #[tokio::test]
    async fn classify_fails_if_never_valid() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"response":"still-not-json"}"#),
            )
            .mount(&server)
            .await;

        let client = LlmClient::new(&server.uri());

        let result = client.classify("template", "summary").await;

        assert!(result.is_err());

        let calls = server.received_requests().await.unwrap().len();

        // depends on max_retries in config (likely 3 or 4 attempts total)
        assert_eq!(calls, client.config.max_retries);
    }
}
