use candle_transformers::models::jina_bert::{BertModel, Config, PositionEmbeddingType};

use anyhow::{Context, Error as E};
use candle_core::{
    DType, Device, Module, Tensor,
    utils::{cuda_is_available, metal_is_available},
};
use candle_nn::VarBuilder;

use hf_hub::{Repo, RepoType, api::sync::Api};
use tokenizers::Tokenizer;
use tracing::info;

use crate::Embedder;

pub struct CandleBert {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    normalize_embeddings: bool,
}
impl Embedder for CandleBert {
    fn embed<S: AsRef<str> + Send + Sync + 'static, T: AsRef<[S]> + 'static>(
        &mut self,
        input: T,
        _batch_size: Option<usize>,
    ) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut result = Vec::new();
        for prompt in input.as_ref() {
            let embeddings = self.get_embeddings(prompt.as_ref())?;
            let r = embeddings.to_vec1::<f32>()?;
            result.push(r);
        }
        Ok(result)
    }
}

impl Default for CandleBert {
    fn default() -> Self {
        Self::new()
    }
}

impl CandleBert {
    pub fn new() -> Self {
        let (model, tokenizer, device) =
            Self::build_model_and_tokenizer(None, None, None, true).unwrap();
        Self {
            model,
            tokenizer,
            device,
            normalize_embeddings: false,
        }
    }

    fn build_model_and_tokenizer(
        model: Option<&str>,
        model_file: Option<&str>,
        tokenizer: Option<&str>,
        cpu: bool,
    ) -> anyhow::Result<(BertModel, Tokenizer, Device)> {
        let model_name = match model {
            Some(model) => model.to_string(),
            None => "jinaai/jina-embeddings-v2-base-en".to_string(),
        };

        let model = match model_file {
            Some(model_file) => std::path::PathBuf::from(model_file),
            None => Api::new()?
                .repo(Repo::new(model_name.to_string(), RepoType::Model))
                .get("model.safetensors")?,
        };
        let tokenizer = match tokenizer {
            Some(file) => std::path::PathBuf::from(file),
            None => Api::new()?
                .repo(Repo::new(model_name.to_string(), RepoType::Model))
                .get("tokenizer.json")?,
        };
        let device = Self::device(cpu)?;
        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer).map_err(E::msg)?;
        let config = Config::new(
            tokenizer.get_vocab_size(true),
            768,
            12,
            12,
            3072,
            candle_nn::Activation::Gelu,
            8192,
            2,
            0.02,
            1e-12,
            0,
            PositionEmbeddingType::Alibi,
        );
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[model], DType::F32, &device)? };
        let model = BertModel::new(vb, &config)?;
        Ok((model, tokenizer, device))
    }

    fn get_embeddings(&self, prompt: &str) -> anyhow::Result<Tensor> {
        let mut binding = self.tokenizer.clone();
        let tokenizer = binding
            .with_padding(None)
            .with_truncation(None)
            .map_err(E::msg)?;
        let tokens = tokenizer
            .encode(prompt, true)
            .map_err(E::msg)?
            .get_ids()
            .to_vec();
        let token_ids = Tensor::new(&tokens[..], &self.device)?.unsqueeze(0)?;
        info!("Loaded and encoded");
        let embeddings = self.model.forward(&token_ids)?;
        let (_n_sentence, n_tokens, _hidden_size) = embeddings.dims3()?;
        let embeddings = (embeddings.sum(1)? / (n_tokens as f64))?;
        info!(?embeddings, "pooled_embeddings");
        let embeddings = if self.normalize_embeddings {
            Self::normalize_l2(&embeddings)?
        } else {
            embeddings
        };
        if self.normalize_embeddings {
            info!(?embeddings, "normalized_embeddings");
        }
        Ok(embeddings)
    }

    pub fn device(cpu: bool) -> anyhow::Result<Device> {
        if cpu {
            Ok(Device::Cpu)
        } else if cuda_is_available() {
            Ok(Device::new_cuda(0)?)
        } else if metal_is_available() {
            Ok(Device::new_metal(0)?)
        } else {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                println!("Running on CPU, to run on GPU(metal), build with Metal features");
            }
            #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
            {
                println!("Running on CPU, to run on GPU, build with CUDA features");
            }
            Ok(Device::Cpu)
        }
    }

    fn normalize_l2(v: &Tensor) -> anyhow::Result<Tensor> {
        v.broadcast_div(&v.sqr()?.sum_keepdim(1)?.sqrt()?)
            .context("normalization")
    }
}
