use super::client::LlmClient;

impl LlmClient {
    pub async fn generate_with_retry<F, T>(&self, prompt: &str, validator: F) -> anyhow::Result<T>
    where
        F: Fn(&str) -> Option<T>,
    {
        let mut attempt = 0;

        let output = loop {
            let output = self.generate(prompt).await?;

            if let Some(valid) = validator(&output) {
                return Ok(valid);
            }

            attempt += 1;

            if attempt >= self.config.max_retries {
                break output;
            }
        };
        Err(anyhow::Error::msg(format!(
            "Failed after {} attempts: {}",
            attempt, output
        )))
    }
}

#[cfg(test)]
mod tests {

    use crate::llm::tests::SequenceResponder;

    use super::*;
    use tracing_test::traced_test;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    #[traced_test]
    async fn generate_with_retry() {
        let server = MockServer::start().await;
        let responses = vec![
            ResponseTemplate::new(200).set_body_string(r#"{"response":"Hello world"}"#),
            ResponseTemplate::new(200).set_body_string(r#"{"response":"Goodbye chickens"}"#),
        ];

        let responder = SequenceResponder::new(responses);

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(responder)
            .mount(&server)
            .await;

        let client = LlmClient::new(&server.uri());

        let result = client
            .generate_with_retry("test prompt", |s| {
                if s.contains("Goodbye") {
                    Some(s.to_string())
                } else {
                    None
                }
            })
            .await
            .unwrap();

        assert_eq!("Goodbye chickens", result);
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    #[traced_test]
    async fn generate_with_retry_failure() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/generate"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"response":"Hello world"}"#),
            )
            .mount(&server)
            .await;

        let client = LlmClient::new(&server.uri());

        let result = client
            .generate_with_retry("test prompt", |s| {
                if s.contains("Goodbye") {
                    Some(s.to_string())
                } else {
                    None
                }
            })
            .await;

        assert!(result.is_err());
        assert_eq!(
            "Failed after 3 attempts: Hello world",
            result.unwrap_err().to_string()
        );

        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }
}
