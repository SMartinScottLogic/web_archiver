use std::fmt::Display;

use reqwest::Response;

use futures_util::StreamExt;

#[allow(dead_code)]
#[derive(Debug)]
pub enum TextLimitedError {
    Reqwest(reqwest::Error),
    TooLarge,
    Utf8(std::string::FromUtf8Error),
}

impl Display for TextLimitedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for TextLimitedError {}

impl From<reqwest::Error> for TextLimitedError {
    fn from(e: reqwest::Error) -> Self {
        TextLimitedError::Reqwest(e)
    }
}

impl From<std::string::FromUtf8Error> for TextLimitedError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        TextLimitedError::Utf8(e)
    }
}

pub async fn text_limited(resp: Response, max_size: usize) -> Result<String, TextLimitedError> {
    // Optional early check
    if let Some(len) = resp.content_length()
        && len > max_size as u64
    {
        return Err(TextLimitedError::TooLarge);
    }

    let mut stream = resp.bytes_stream();
    let mut data = Vec::with_capacity(usize::min(max_size, 8192));

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;

        if data.len() + chunk.len() > max_size {
            return Err(TextLimitedError::TooLarge);
        }

        data.extend_from_slice(&chunk);
    }

    Ok(String::from_utf8(data)?)
}

pub trait ResponseExt {
    fn text_limited(
        self,
        max: usize,
    ) -> impl std::future::Future<Output = Result<String, TextLimitedError>>;
}

impl ResponseExt for Response {
    fn text_limited(
        self,
        max: usize,
    ) -> impl std::future::Future<Output = Result<String, TextLimitedError>> {
        text_limited(self, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get};
    use tokio::task;

    async fn spawn_server(body: &'static [u8]) -> String {
        let app = Router::new().route("/", get(move || async move { body }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        task::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn test_within_limit() {
        let url = spawn_server(b"hello world").await;

        let resp = reqwest::get(&url).await.unwrap();
        let text = text_limited(resp, 100).await.unwrap();

        assert_eq!(text, "hello world");
    }

    #[tokio::test]
    async fn test_exact_limit() {
        let body = b"12345";
        let url = spawn_server(body).await;

        let resp = reqwest::get(&url).await.unwrap();
        let text = text_limited(resp, body.len()).await.unwrap();

        assert_eq!(text, "12345");
    }

    #[tokio::test]
    async fn test_exceeds_limit_stream() {
        let body = b"this is definitely too long";
        let url = spawn_server(body).await;

        let resp = reqwest::get(&url).await.unwrap();
        let result = text_limited(resp, 10).await;

        assert!(matches!(result, Err(TextLimitedError::TooLarge)));
    }

    #[tokio::test]
    async fn test_content_length_exceeds_limit() {
        let body = b"large body here";
        let url = spawn_server(body).await;

        let client = reqwest::Client::new();
        let resp = client.get(&url).send().await.unwrap();

        // smaller than actual content-length → should early fail
        let result = text_limited(resp, 5).await;

        assert!(matches!(result, Err(TextLimitedError::TooLarge)));
    }

    #[tokio::test]
    async fn test_invalid_utf8() {
        // Invalid UTF-8 bytes
        let body = b"\xFF\xFE\xFD";
        let url = spawn_server(body).await;

        let resp = reqwest::get(&url).await.unwrap();
        let result = text_limited(resp, 100).await;

        assert!(matches!(result, Err(TextLimitedError::Utf8(_))));
    }

    #[tokio::test]
    async fn test_response_ext_trait() {
        let url = spawn_server(b"trait works").await;

        let resp = reqwest::get(&url).await.unwrap();
        let text = resp.text_limited(100).await.unwrap();

        assert_eq!(text, "trait works");
    }

    #[tokio::test]
    async fn test_empty_body() {
        let url = spawn_server(b"").await;

        let resp = reqwest::get(&url).await.unwrap();
        let text = text_limited(resp, 10).await.unwrap();

        assert_eq!(text, "");
    }
}
