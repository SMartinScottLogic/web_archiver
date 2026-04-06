use mockall::automock;

use crate::candle_bert::CandleBert;

pub mod candle_bert;

#[automock]
#[allow(clippy::needless_lifetimes)]
pub trait Embedder {
    fn embed<S: AsRef<str> + Send + Sync + 'static, T: AsRef<[S]> + 'static>(
        &mut self,
        input: T,
        batch_size: Option<usize>,
    ) -> anyhow::Result<Vec<Vec<f32>>>;
}

pub fn create_default_embedder() -> impl Embedder {
    CandleBert::new()
}
