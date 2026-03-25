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

impl LlmClient {
    pub async fn summarise(&self, template: &str, text: &str) -> anyhow::Result<Vec<String>> {
        let prompt = fill_prompt(template, &[("chunk", text)]);
        self.generate_with_retry(&prompt, |output| {
            if !validate_bullets(output, 3, 7) {
                return None;
            }

            let bullets = output
                .lines()
                .filter(|l| l.trim().starts_with(['-', '*']))
                .map(|l| l.trim().trim_start_matches("- ").to_string())
                .collect::<Vec<_>>();

            Some(bullets)
        })
        .await
    }
}
