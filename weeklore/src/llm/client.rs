use serde::{Deserialize, Serialize};

use crate::llm::{prompt::fill_prompt, validate::validate_bullets};

#[derive(Clone)]
pub struct LlmConfig {
    pub endpoint: String,
    pub model: String,
    pub temperature: f32,
    pub max_retries: usize,
}

pub struct LlmClient {
    client: reqwest::Client,
    pub config: LlmConfig,
}

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    temperature: f32,
    think: bool,
    stream: bool,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

impl LlmClient {
    pub fn new(llm_host: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            config: LlmConfig {
                endpoint: format!("{}/api/generate", llm_host),
                //model: "gpt-oss:20b".into(),
                //model: "huihui_ai/deepseek-r1-abliterated:latest".into(),
                model: "goekdenizguelmez/JOSIEFIED-Qwen3:30b".into(),
                temperature: 0.1,
                max_retries: 3,
            },
        }
    }

    pub(crate) async fn generate(&self, prompt: &str) -> anyhow::Result<String> {
        let res = self
            .client
            .post(&self.config.endpoint)
            .json(&GenerateRequest {
                model: &self.config.model,
                prompt,
                temperature: self.config.temperature,
                think: false,
                stream: false,
            })
            .send()
            .await?;

        let text = res.text().await?;

        let parsed: Result<GenerateResponse, _> = serde_json::from_str(&text);

        Ok(parsed
            .map(|p| p.response)
            .unwrap_or(text)
            .trim()
            .to_string())
    }
}

pub trait LlmSummarise {
    async fn summarise(&self, template: &str, text: &str) -> anyhow::Result<Vec<String>>;
}

impl LlmSummarise for LlmClient {
    async fn summarise(&self, template: &str, text: &str) -> anyhow::Result<Vec<String>> {
        let prompt = fill_prompt(template, &[("chunk", text)]);
        self.generate_with_retry(&prompt, |output| {
            if !validate_bullets(output, 3, 7) {
                return None;
            }

            let bullets = output
                .lines()
                .filter(|l| l.trim().starts_with(['-', '*']))
                .map(|l| {
                    l.trim()
                        .trim_start_matches(['-', '*'])
                        .trim_start()
                        .to_string()
                })
                .collect::<Vec<_>>();

            Some(bullets)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use crate::llm::tests::SequenceResponder;

    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use tracing_test::traced_test;

    #[tokio::test]
    #[traced_test]
    async fn test_generate_parses_json_response() {
        let server = MockServer::start().await;
        // First response (invalid → triggers retry)
        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"response":"Hello world"}"#),
            )
            .expect(1) // only allow once
            .mount(&server)
            .await;

        let client = LlmClient::new(&server.uri());

        let result = client.generate("test prompt").await.unwrap();

        assert_eq!(result, "Hello world");
    }

    #[tokio::test]
    async fn summarise_success_first_try() {
        let server = MockServer::start().await;

        let body = r#"{"response":"- one\n- two\n- three"}"#;

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let client = LlmClient::new(&server.uri());

        let result = client.summarise("template", "input text").await.unwrap();

        assert_eq!(result, vec!["one", "two", "three"]);

        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn summarise_retries_until_valid() {
        let server = MockServer::start().await;

        let responses = vec![
            // First: invalid (not enough bullets / wrong format)
            ResponseTemplate::new(200).set_body_string(r#"{"response":"not bullets"}"#),
            // Second: valid bullets
            ResponseTemplate::new(200).set_body_string(r#"{"response":"- one\n- two\n- three"}"#),
        ];

        let responder = SequenceResponder::new(responses);

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(responder)
            .mount(&server)
            .await;

        let client = LlmClient::new(&server.uri());

        let result = client.summarise("template", "input text").await.unwrap();

        assert_eq!(result, vec!["one", "two", "three"]);

        let calls = server.received_requests().await.unwrap().len();
        assert_eq!(calls, 2);
    }

    #[tokio::test]
    async fn summarise_fails_if_never_valid() {
        let server = MockServer::start().await;

        // Always invalid
        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"response":"still wrong"}"#),
            )
            .mount(&server)
            .await;

        let client = LlmClient::new(&server.uri());

        let result = client.summarise("template", "input text").await;

        assert!(result.is_err());

        let calls = server.received_requests().await.unwrap().len();
        assert_eq!(calls, client.config.max_retries);
    }

    #[tokio::test]
    async fn summarise_accepts_star_bullets() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"response":"* one\n* two\n* three"}"#),
            )
            .mount(&server)
            .await;

        let client = LlmClient::new(&server.uri());

        let result = client.summarise("template", "input text").await.unwrap();

        assert_eq!(result, vec!["one", "two", "three"]);
    }

    #[tokio::test]
    async fn summarise_rejects_too_many_bullets() {
        let server = MockServer::start().await;

        let body = r#"{"response":"- 1\n- 2\n- 3\n- 4\n- 5\n- 6\n- 7\n- 8"}"#;

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let client = LlmClient::new(&server.uri());

        let result = client.summarise("template", "input text").await;

        assert!(result.is_err());
    }
}
