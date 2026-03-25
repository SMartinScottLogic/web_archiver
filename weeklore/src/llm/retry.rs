use super::client::LlmClient;

impl LlmClient {
    pub async fn generate_with_retry<F, T>(&self, prompt: &str, validator: F) -> anyhow::Result<T>
    where
        F: Fn(&str) -> Option<T>,
    {
        let mut attempt = 0;

        loop {
            let output = self.generate(prompt).await?;

            if let Some(valid) = validator(&output) {
                return Ok(valid);
            }

            attempt += 1;

            if attempt >= self.config.max_retries {
                anyhow::bail!("Failed after {} attempts: {}", attempt, output);
            }
        }
    }
}
