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
