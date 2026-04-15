pub mod client;
pub mod prompt;
pub mod retry;
pub mod validate;

pub mod classify;

pub use classify::Category;
pub use client::LlmClient;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use wiremock::Request;
    use wiremock::{Respond, ResponseTemplate};

    pub(crate) struct SequenceResponder {
        responses: Arc<Mutex<Vec<ResponseTemplate>>>,
    }

    impl SequenceResponder {
        pub(crate) fn new(responses: Vec<ResponseTemplate>) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses)),
            }
        }
    }

    impl Respond for SequenceResponder {
        fn respond(&self, _request: &Request) -> ResponseTemplate {
            let mut responses = self.responses.lock().unwrap();

            if responses.len() > 1 {
                responses.remove(0)
            } else {
                // keep returning last response if exhausted
                responses[0].clone()
            }
        }
    }
}
