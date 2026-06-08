use std::path::{Path, PathBuf};
use std::sync::Mutex;

use calyx_core::{CalyxError, Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use hf_hub::api::sync::ApiBuilder;
use tokenizers::{Tokenizer, TruncationParams};

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::runtime::common::{
    DEFAULT_MAX_TOKENS, default_hf_cache_root, hash_files, normalize_unit, text_from_input,
};

pub const DEFAULT_CANDLE_MODEL: &str = "sentence-transformers/all-MiniLM-L6-v2";

pub struct CandleLens {
    id: LensId,
    dim: u32,
    contract: FrozenLensContract,
    files: CandleModelFiles,
    device_policy: CandleDevicePolicy,
    max_tokens: usize,
    tokenizer: Tokenizer,
    model: Mutex<BertModel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandleModelFiles {
    pub cache_dir: PathBuf,
    pub model_id: String,
    pub config: PathBuf,
    pub tokenizer: PathBuf,
    pub weights: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandleDevicePolicy {
    CpuExplicit,
    CudaFailLoud { ordinal: usize },
}

impl CandleDevicePolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CpuExplicit => "cpu_explicit,no_cuda",
            Self::CudaFailLoud { .. } => "cuda,error_on_failure,no_cpu_fallback",
        }
    }
}

impl CandleLens {
    pub fn all_minilm_l6_v2(name: impl Into<String>) -> Result<Self> {
        Self::from_hf_cache(name, default_hf_cache_root())
    }

    pub fn all_minilm_l6_v2_cuda_fail_loud(name: impl Into<String>) -> Result<Self> {
        Self::from_hf_cache_with_device_policy(
            name,
            default_hf_cache_root(),
            CandleDevicePolicy::CudaFailLoud { ordinal: 0 },
        )
    }

    pub fn from_hf_cache(name: impl Into<String>, cache_dir: impl Into<PathBuf>) -> Result<Self> {
        Self::from_hf_cache_with_device_policy(name, cache_dir, CandleDevicePolicy::CpuExplicit)
    }

    pub fn from_hf_cache_with_device_policy(
        name: impl Into<String>,
        cache_dir: impl Into<PathBuf>,
        device_policy: CandleDevicePolicy,
    ) -> Result<Self> {
        Self::from_model(
            name,
            DEFAULT_CANDLE_MODEL,
            cache_dir.into(),
            DEFAULT_MAX_TOKENS,
            device_policy,
        )
    }

    pub fn from_model(
        name: impl Into<String>,
        model_id: impl Into<String>,
        cache_dir: PathBuf,
        max_tokens: usize,
        device_policy: CandleDevicePolicy,
    ) -> Result<Self> {
        let name = name.into();
        let model_id = model_id.into();
        let files = fetch_files(&cache_dir, &model_id)?;
        let config = read_config(&files.config)?;
        let tokenizer = read_tokenizer(&files.tokenizer, max_tokens)?;
        let model = read_model(&files.weights, &config, device_policy)?;
        let weights_sha256 = hash_files(&[
            files.config.clone(),
            files.tokenizer.clone(),
            files.weights.clone(),
        ])?;
        let max_tokens_text = max_tokens.to_string();
        let corpus_hash = sha256_digest(&[
            b"candle-local-bert-mean-pool-v1",
            model_id.as_bytes(),
            max_tokens_text.as_bytes(),
        ]);
        let dim = u32::try_from(config.hidden_size).map_err(|_| {
            CalyxError::lens_dim_mismatch(format!(
                "candle hidden size {} exceeds u32",
                config.hidden_size
            ))
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
            device_policy,
            max_tokens,
            tokenizer,
            model: Mutex::new(model),
        })
    }

    pub fn contract(&self) -> &FrozenLensContract {
        &self.contract
    }

    pub fn files(&self) -> &CandleModelFiles {
        &self.files
    }

    pub const fn device_policy(&self) -> CandleDevicePolicy {
        self.device_policy
    }

    pub const fn max_tokens(&self) -> usize {
        self.max_tokens
    }
}

impl Lens for CandleLens {
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
        let text = text_from_input(self, input)?;
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|err| CalyxError::lens_dim_mismatch(format!("tokenize failed: {err}")))?;
        let ids = encoding.get_ids().to_vec();
        let mask = encoding.get_attention_mask().to_vec();
        let seq = ids.len();
        if seq == 0 {
            return Err(CalyxError::lens_dim_mismatch(
                "candle tokenizer returned no tokens",
            ));
        }

        let model = self.model.lock().map_err(|_| {
            CalyxError::lens_unreachable("candle model mutex was poisoned during inference")
        })?;
        let device = model.device.clone();
        let input_ids = Tensor::from_vec(ids, (1, seq), &device).map_err(candle_error)?;
        let token_type_ids =
            Tensor::from_vec(vec![0_u32; seq], (1, seq), &device).map_err(candle_error)?;
        let attention_mask =
            Tensor::from_vec(mask.clone(), (1, seq), &device).map_err(candle_error)?;
        let hidden = model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))
            .map_err(candle_error)?;
        let rows = hidden.to_vec3::<f32>().map_err(candle_error)?;
        let first = rows.first().ok_or_else(|| {
            CalyxError::lens_dim_mismatch("candle model returned empty batch output")
        })?;
        let mut data = mean_pool(first, &mask, self.dim as usize)?;
        normalize_unit(&mut data)?;
        Ok(SlotVector::Dense {
            dim: self.dim,
            data,
        })
    }
}

fn fetch_files(cache_dir: &Path, model_id: &str) -> Result<CandleModelFiles> {
    let api = ApiBuilder::new()
        .with_cache_dir(cache_dir.to_path_buf())
        .with_progress(false)
        .build()
        .map_err(|err| CalyxError::lens_unreachable(format!("HF API init failed: {err}")))?;
    let repo = api.model(model_id.to_string());
    let config = repo
        .get("config.json")
        .map_err(|err| CalyxError::lens_unreachable(format!("fetch config.json failed: {err}")))?;
    let tokenizer = repo.get("tokenizer.json").map_err(|err| {
        CalyxError::lens_unreachable(format!("fetch tokenizer.json failed: {err}"))
    })?;
    let weights = repo.get("model.safetensors").map_err(|err| {
        CalyxError::lens_unreachable(format!("fetch model.safetensors failed: {err}"))
    })?;
    Ok(CandleModelFiles {
        cache_dir: cache_dir.to_path_buf(),
        model_id: model_id.to_string(),
        config,
        tokenizer,
        weights,
    })
}

fn read_config(path: &Path) -> Result<Config> {
    let bytes = std::fs::read(path).map_err(|err| {
        CalyxError::lens_unreachable(format!("read BERT config {} failed: {err}", path.display()))
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|err| CalyxError::lens_unreachable(format!("parse BERT config failed: {err}")))
}

fn read_tokenizer(path: &Path, max_tokens: usize) -> Result<Tokenizer> {
    let mut tokenizer = Tokenizer::from_file(path)
        .map_err(|err| CalyxError::lens_unreachable(format!("load tokenizer failed: {err}")))?;
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: max_tokens,
            ..Default::default()
        }))
        .map_err(|err| CalyxError::lens_dim_mismatch(format!("set truncation failed: {err}")))?;
    Ok(tokenizer)
}

fn read_model(
    weights: &Path,
    config: &Config,
    device_policy: CandleDevicePolicy,
) -> Result<BertModel> {
    let device = candle_device(device_policy)?;
    let paths = [weights];
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&paths, DType::F32, &device) }
        .map_err(candle_error)?;
    BertModel::load(vb, config).map_err(candle_error)
}

fn candle_device(policy: CandleDevicePolicy) -> Result<Device> {
    match policy {
        CandleDevicePolicy::CpuExplicit => Ok(Device::Cpu),
        CandleDevicePolicy::CudaFailLoud { ordinal } => candle_cuda_device(ordinal),
    }
}

#[cfg(feature = "candle-cuda")]
fn candle_cuda_device(ordinal: usize) -> Result<Device> {
    Device::new_cuda(ordinal)
        .map_err(|err| CalyxError::lens_unreachable(format!("candle CUDA init failed: {err}")))
}

#[cfg(not(feature = "candle-cuda"))]
fn candle_cuda_device(_ordinal: usize) -> Result<Device> {
    Err(CalyxError::lens_unreachable(
        "candle CUDA requested but calyx-registry was built without feature `candle-cuda`",
    ))
}

fn mean_pool(tokens: &[Vec<f32>], mask: &[u32], dim: usize) -> Result<Vec<f32>> {
    let mut out = vec![0.0_f32; dim];
    let mut count = 0_u32;
    for (row, keep) in tokens.iter().zip(mask) {
        if *keep == 0 {
            continue;
        }
        if row.len() != dim {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "candle token dim {} != expected {dim}",
                row.len()
            )));
        }
        for (dst, value) in out.iter_mut().zip(row) {
            *dst += *value;
        }
        count += 1;
    }
    if count == 0 {
        return Err(CalyxError::lens_dim_mismatch(
            "candle attention mask selected no tokens",
        ));
    }
    let inv = 1.0 / count as f32;
    for value in &mut out {
        *value *= inv;
    }
    Ok(out)
}

fn candle_error(err: candle_core::Error) -> CalyxError {
    CalyxError::lens_unreachable(format!("candle runtime failed: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::Path;

    #[test]
    fn mean_pool_uses_attention_mask() {
        let tokens = vec![vec![1.0, 3.0], vec![5.0, 9.0]];

        let pooled = mean_pool(&tokens, &[1, 0], 2).unwrap();

        assert_eq!(pooled, vec![1.0, 3.0]);
    }

    #[test]
    fn mean_pool_rejects_wrong_dim() {
        let error = mean_pool(&[vec![1.0]], &[1], 2).unwrap_err();

        assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");
    }

    #[test]
    fn candle_device_policy_reports_cpu_and_cuda_truth() {
        assert_eq!(
            CandleDevicePolicy::CpuExplicit.as_str(),
            "cpu_explicit,no_cuda"
        );
        assert!(matches!(
            candle_device(CandleDevicePolicy::CpuExplicit).unwrap(),
            Device::Cpu
        ));
        let cuda_feature = cfg!(feature = "candle-cuda");
        let cuda_result = candle_device(CandleDevicePolicy::CudaFailLoud { ordinal: 0 });
        let cuda_error = if cuda_feature {
            assert!(
                cuda_result.is_ok() || cuda_result.as_ref().unwrap_err().message.contains("CUDA")
            );
            cuda_result.err()
        } else {
            let error = cuda_result.expect_err("cuda feature is not compiled by default");
            assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
            assert!(error.message.contains("without feature `candle-cuda`"));
            Some(error)
        };

        if let Some(root) = std::env::var_os("CALYX_FSV_ROOT") {
            write_device_policy_readback(Path::new(&root), cuda_feature, cuda_error);
        }
    }

    fn write_device_policy_readback(
        root: &Path,
        cuda_feature: bool,
        cuda_error: Option<CalyxError>,
    ) {
        fs::create_dir_all(root).unwrap();
        let readback = json!({
            "default_policy": CandleDevicePolicy::CpuExplicit.as_str(),
            "cuda_policy": CandleDevicePolicy::CudaFailLoud { ordinal: 0 }.as_str(),
            "candle_cuda_feature_compiled": cuda_feature,
            "cuda_fail_loud_error_code": cuda_error.as_ref().map(|error| error.code),
            "cuda_fail_loud_error_message": cuda_error.as_ref().map(|error| error.message.as_str()),
        });
        fs::write(
            root.join("candle-device-policy-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
    }

    #[test]
    #[ignore = "requires aiwonder HF cache/network and downloads all-MiniLM weights"]
    fn candle_all_minilm_aiwonder_fsv() {
        let lens = CandleLens::all_minilm_l6_v2("candle-aiwonder-fsv").unwrap();
        println!("CANDLE_FSV_DEVICE_POLICY={}", lens.device_policy().as_str());
        let input = Input::new(Modality::Text, b"Calyx PH19 candle local probe".to_vec());
        let vector = lens.measure(&input).unwrap();

        if let SlotVector::Dense { dim, data } = vector {
            println!("CANDLE_FSV_CACHE={}", lens.files().cache_dir.display());
            println!("CANDLE_FSV_WEIGHTS={}", lens.files().weights.display());
            println!("CANDLE_FSV_DIM={dim}");
            println!("CANDLE_FSV_FIRST3={:?}", &data[..3]);
            let norm = data.iter().map(|v| v * v).sum::<f32>().sqrt();
            println!("CANDLE_FSV_NORM={norm:.8}");
            assert!((norm - 1.0).abs() < 1.0e-3);
        } else {
            panic!("expected dense candle vector");
        }
    }

    #[test]
    #[ignore = "requires aiwonder HF cache/network and downloads all-MiniLM weights"]
    fn candle_dim_guard_aiwonder_fsv() {
        let lens = CandleLens::all_minilm_l6_v2("candle-aiwonder-dim-guard").unwrap();
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

        println!("CANDLE_DIM_GUARD_ERROR={}", error.code);
        assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");

        let empty = lens
            .measure(&Input::new(Modality::Text, Vec::new()))
            .unwrap();
        if let SlotVector::Dense { dim, data } = empty {
            let norm = data.iter().map(|v| v * v).sum::<f32>().sqrt();
            println!("CANDLE_EMPTY_DIM={dim}");
            println!("CANDLE_EMPTY_NORM={norm:.8}");
            println!("CANDLE_EMPTY_FIRST3={:?}", &data[..3]);
            assert!((norm - 1.0).abs() < 1.0e-3);
        } else {
            panic!("expected dense empty candle vector");
        }

        let invalid = lens
            .measure(&Input::new(Modality::Text, vec![0xff]))
            .unwrap_err();
        println!("CANDLE_INVALID_UTF8_ERROR={}", invalid.code);
        assert_eq!(invalid.code, "CALYX_LENS_DIM_MISMATCH");

        let wrong_modality = lens
            .measure(&Input::new(Modality::Image, b"pixels".to_vec()))
            .unwrap_err();
        println!("CANDLE_WRONG_MODALITY_ERROR={}", wrong_modality.code);
        assert_eq!(wrong_modality.code, "CALYX_LENS_DIM_MISMATCH");
    }
}
