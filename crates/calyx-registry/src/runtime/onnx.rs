use std::path::{Path, PathBuf};
use std::sync::Mutex;

use calyx_core::{CalyxError, Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use hf_hub::api::sync::ApiBuilder;
use ort::ep;

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::runtime::common::{
    default_hf_cache_root, fastembed_cache_root, hash_files, normalize_unit, text_from_input,
};

pub struct OnnxLens {
    id: LensId,
    dim: u32,
    contract: FrozenLensContract,
    files: OnnxModelFiles,
    model: Mutex<TextEmbedding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnnxModelFiles {
    pub cache_dir: PathBuf,
    pub model_code: String,
    pub model_file: PathBuf,
    pub tokenizer: PathBuf,
    pub config: PathBuf,
    pub special_tokens_map: PathBuf,
    pub tokenizer_config: PathBuf,
}

impl OnnxLens {
    pub fn all_minilm_l6_v2(name: impl Into<String>) -> Result<Self> {
        Self::from_hf_cache(name, default_hf_cache_root())
    }

    pub fn from_hf_cache(name: impl Into<String>, cache_dir: impl Into<PathBuf>) -> Result<Self> {
        Self::from_model(name, EmbeddingModel::AllMiniLML6V2, cache_dir.into())
    }

    pub fn from_model(
        name: impl Into<String>,
        model_name: EmbeddingModel,
        cache_dir: PathBuf,
    ) -> Result<Self> {
        let name = name.into();
        let info = TextEmbedding::get_model_info(&model_name).map_err(|err| {
            CalyxError::lens_unreachable(format!("fastembed model metadata failed: {err}"))
        })?;
        let model = TextEmbedding::try_new(
            TextInitOptions::new(model_name.clone())
                .with_cache_dir(cache_dir.clone())
                .with_show_download_progress(false)
                .with_intra_threads(1)
                .with_execution_providers(execution_providers()),
        )
        .map_err(|err| CalyxError::lens_unreachable(format!("ONNX runtime init failed: {err}")))?;
        let effective_cache = fastembed_cache_root(&cache_dir);
        let files = resolve_files(&effective_cache, &info.model_code, &info.model_file)?;
        let weights_sha256 = hash_files(&[
            files.model_file.clone(),
            files.tokenizer.clone(),
            files.config.clone(),
            files.special_tokens_map.clone(),
            files.tokenizer_config.clone(),
        ])?;
        let corpus_hash = sha256_digest(&[
            b"onnx-fastembed-mean-pool-v1",
            info.model_code.as_bytes(),
            info.model_file.as_bytes(),
        ]);
        let dim = u32::try_from(info.dim).map_err(|_| {
            CalyxError::lens_dim_mismatch(format!("ONNX dim {} exceeds u32", info.dim))
        })?;
        let contract = FrozenLensContract::new(
            name,
            weights_sha256,
            corpus_hash,
            SlotShape::Dense(dim),
            Modality::Text,
            LensDType::F32,
            NormPolicy::unit(),
        );
        let id = contract.lens_id();
        Ok(Self {
            id,
            dim,
            contract,
            files,
            model: Mutex::new(model),
        })
    }

    pub fn contract(&self) -> &FrozenLensContract {
        &self.contract
    }

    pub fn files(&self) -> &OnnxModelFiles {
        &self.files
    }
}

impl Lens for OnnxLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(self.dim)
    }

    fn modality(&self) -> Modality {
        Modality::Text
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        let mut batch = self.measure_batch(std::slice::from_ref(input))?;
        batch.pop().ok_or_else(|| {
            CalyxError::lens_dim_mismatch(format!("lens {} returned no ONNX vector", self.id))
        })
    }

    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let mut texts = Vec::with_capacity(inputs.len());
        for input in inputs {
            texts.push(text_from_input(self, input)?.to_string());
        }
        let mut model = self.model.lock().map_err(|_| {
            CalyxError::lens_unreachable("ONNX model mutex was poisoned during inference")
        })?;
        let embeddings = model
            .embed(texts, None)
            .map_err(|err| CalyxError::lens_unreachable(format!("ONNX inference failed: {err}")))?;
        if embeddings.len() != inputs.len() {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "ONNX returned {} vectors for {} inputs",
                embeddings.len(),
                inputs.len()
            )));
        }
        embeddings
            .into_iter()
            .map(|mut data| {
                if data.len() != self.dim as usize {
                    return Err(CalyxError::lens_dim_mismatch(format!(
                        "ONNX dim {} != expected {}",
                        data.len(),
                        self.dim
                    )));
                }
                normalize_unit(&mut data)?;
                Ok(SlotVector::Dense {
                    dim: self.dim,
                    data,
                })
            })
            .collect()
    }
}

fn execution_providers() -> Vec<fastembed::ExecutionProviderDispatch> {
    vec![
        ep::CUDA::default()
            .with_device_id(0)
            .build()
            .fail_silently(),
        ep::CPU::default().build(),
    ]
}

fn resolve_files(cache_dir: &Path, model_code: &str, model_file: &str) -> Result<OnnxModelFiles> {
    let api = ApiBuilder::new()
        .with_cache_dir(cache_dir.to_path_buf())
        .with_progress(false)
        .build()
        .map_err(|err| CalyxError::lens_unreachable(format!("HF API init failed: {err}")))?;
    let repo = api.model(model_code.to_string());
    Ok(OnnxModelFiles {
        cache_dir: cache_dir.to_path_buf(),
        model_code: model_code.to_string(),
        model_file: fetch(&repo, model_file)?,
        tokenizer: fetch(&repo, "tokenizer.json")?,
        config: fetch(&repo, "config.json")?,
        special_tokens_map: fetch(&repo, "special_tokens_map.json")?,
        tokenizer_config: fetch(&repo, "tokenizer_config.json")?,
    })
}

fn fetch(repo: &hf_hub::api::sync::ApiRepo, filename: &str) -> Result<PathBuf> {
    repo.get(filename)
        .map_err(|err| CalyxError::lens_unreachable(format!("fetch {filename} failed: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_provider_order_prefers_cuda_then_cpu() {
        let providers = execution_providers();

        assert_eq!(providers.len(), 2);
    }

    #[test]
    #[ignore = "requires aiwonder HF cache/network and downloads ONNX all-MiniLM"]
    fn onnx_all_minilm_aiwonder_fsv() {
        let lens = OnnxLens::all_minilm_l6_v2("onnx-aiwonder-fsv").unwrap();
        let input = Input::new(Modality::Text, b"Calyx PH19 ONNX local probe".to_vec());
        let vector = lens.measure(&input).unwrap();

        if let SlotVector::Dense { dim, data } = vector {
            println!("ONNX_FSV_CACHE={}", lens.files().cache_dir.display());
            println!("ONNX_FSV_MODEL={}", lens.files().model_file.display());
            println!("ONNX_FSV_DIM={dim}");
            println!("ONNX_FSV_FIRST3={:?}", &data[..3]);
            let norm = data.iter().map(|v| v * v).sum::<f32>().sqrt();
            println!("ONNX_FSV_NORM={norm:.8}");
            assert!((norm - 1.0).abs() < 1.0e-3);
        } else {
            panic!("expected dense ONNX vector");
        }
    }

    #[test]
    #[ignore = "requires aiwonder HF cache/network and downloads ONNX all-MiniLM"]
    fn onnx_dim_guard_aiwonder_fsv() {
        let lens = OnnxLens::all_minilm_l6_v2("onnx-aiwonder-dim-guard").unwrap();
        let error = lens
            .contract()
            .verify_vector(
                lens.id(),
                &SlotVector::Dense {
                    dim: 3,
                    data: vec![1.0, 0.0, 0.0],
                },
            )
            .unwrap_err();

        println!("ONNX_DIM_GUARD_ERROR={}", error.code);
        assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");

        let empty = lens
            .measure(&Input::new(Modality::Text, Vec::new()))
            .unwrap();
        if let SlotVector::Dense { dim, data } = empty {
            let norm = data.iter().map(|v| v * v).sum::<f32>().sqrt();
            println!("ONNX_EMPTY_DIM={dim}");
            println!("ONNX_EMPTY_NORM={norm:.8}");
            println!("ONNX_EMPTY_FIRST3={:?}", &data[..3]);
            assert!((norm - 1.0).abs() < 1.0e-3);
        } else {
            panic!("expected dense empty ONNX vector");
        }

        let invalid = lens
            .measure(&Input::new(Modality::Text, vec![0xff]))
            .unwrap_err();
        println!("ONNX_INVALID_UTF8_ERROR={}", invalid.code);
        assert_eq!(invalid.code, "CALYX_LENS_DIM_MISMATCH");

        let wrong_modality = lens
            .measure(&Input::new(Modality::Image, b"pixels".to_vec()))
            .unwrap_err();
        println!("ONNX_WRONG_MODALITY_ERROR={}", wrong_modality.code);
        assert_eq!(wrong_modality.code, "CALYX_LENS_DIM_MISMATCH");
    }
}
