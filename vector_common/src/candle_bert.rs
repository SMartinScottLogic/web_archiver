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

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::{Device, Tensor};

    /// Helper to create a simple tensor
    fn tensor_2d(data: &[f32], rows: usize, cols: usize) -> Tensor {
        Tensor::from_slice(data, (rows, cols), &Device::Cpu).unwrap()
    }

    #[test]
    fn test_device_cpu_forced() {
        let device = CandleBert::device(true).unwrap();
        match device {
            Device::Cpu => {}
            _ => panic!("Expected CPU device when cpu=true"),
        }
    }

    #[test]
    fn test_device_fallback() {
        // When cpu = false, we should still get a valid device (CPU/GPU)
        let device = CandleBert::device(false);
        // We can't guarantee CUDA/Metal exists, but it must not error
        match device {
            Ok(Device::Cpu) | Ok(Device::Cuda(_)) | Ok(Device::Metal(_)) => {}
            _ => panic!("Unexpected device type"),
        }
    }

    #[test]
    fn test_normalize_l2_basic() {
        let t = tensor_2d(&[3.0, 4.0], 1, 2);
        let normalized = CandleBert::normalize_l2(&t).unwrap();

        let v = normalized.to_vec2::<f32>().unwrap();
        let norm = (v[0][0].powi(2) + v[0][1].powi(2)).sqrt();

        assert!((norm - 1.0).abs() < 1e-5, "Vector is not unit length");
    }

    #[test]
    fn test_normalize_l2_zero_vector() {
        let t = tensor_2d(&[0.0, 0.0], 1, 2);
        let result = CandleBert::normalize_l2(&t);
        if let Ok(r) = &result {
            let nan = r
                .to_vec2::<f32>()
                .iter()
                .flat_map(|v| v.iter())
                .flat_map(|v| v.iter())
                .all(|v| v.is_nan());
            assert!(nan, "Zero vector should normalize to NaN");
        } else {
            assert!(result.is_err(), "Zero vector should fail normalization");
        }
    }

    #[test]
    fn test_normalize_l2_multiple_rows() {
        let t = tensor_2d(&[3.0, 4.0, 1.0, 2.0], 2, 2);

        let normalized = CandleBert::normalize_l2(&t).unwrap();
        let v = normalized.to_vec2::<f32>().unwrap();

        for row in v {
            let norm = (row[0].powi(2) + row[1].powi(2)).sqrt();
            assert!((norm - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn test_embed_trait_empty_input() {
        struct DummyEmbedder;

        impl Embedder for DummyEmbedder {
            fn embed<S: AsRef<str> + Send + Sync + 'static, T: AsRef<[S]> + 'static>(
                &mut self,
                input: T,
                _batch_size: Option<usize>,
            ) -> anyhow::Result<Vec<Vec<f32>>> {
                Ok(input.as_ref().iter().map(|_| vec![0.0]).collect())
            }
        }

        let mut embedder = DummyEmbedder;
        let result = embedder.embed(Vec::<String>::new(), None).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    #[ignore] // Requires model download
    fn test_default_constructor() {
        // This may download a model — consider marking as ignored if needed
        let model = CandleBert::default();
        assert!(!model.normalize_embeddings);
    }

    #[test]
    #[ignore] // Requires model download + heavy compute
    fn test_real_embedding_shape() {
        let mut model = CandleBert::new();
        let result = model.embed(vec!["hello world"], None).unwrap();

        assert_eq!(result.len(), 1);
        assert!(!result[0].is_empty());
    }
}
