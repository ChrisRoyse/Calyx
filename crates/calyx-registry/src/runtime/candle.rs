use std::path::{Path, PathBuf};
use std::sync::Mutex;

use calyx_core::{CalyxError, Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use candle_core::{DType, Tensor};
use candle_transformers::models::bert::BertModel;
use tokenizers::Tokenizer;

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::runtime::common::{
    DEFAULT_MAX_TOKENS, default_hf_cache_root, hash_files, text_from_input,
};
use crate::spec::{LensRuntime, LensSpec};

pub const DEFAULT_CANDLE_MODEL: &str = "sentence-transformers/all-MiniLM-L6-v2";

mod load;
mod options;
mod pooling;

#[cfg(test)]
use load::{
    HALF_CUDA_MIN_LAYER_NORM_EPS, candle_device, candle_error_message, stabilize_half_cuda_config,
};
use load::{
    candle_error, config_invalid, ensure_file, fetch_files, needs_f32_finite_replay, read_config,
    read_model, read_tokenizer,
};
pub use options::{
    CandleDevicePolicy, CandleFileSpec, CandleModelFiles, CandlePoolingPolicy, CandlePrecision,
};
#[cfg(test)]
use pooling::mean_pool;
use pooling::{apply_norm, pool_tokens};

pub struct CandleLens {
    id: LensId,
    dim: u32,
    contract: FrozenLensContract,
    files: CandleModelFiles,
    device_policy: CandleDevicePolicy,
    precision: CandlePrecision,
    pooling: CandlePoolingPolicy,
    max_tokens: usize,
    tokenizer: Tokenizer,
    model: Mutex<BertModel>,
    finite_replay_model: Option<Mutex<BertModel>>,
    finite_replay_precision: Option<CandlePrecision>,
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
        Self::from_model_with_options(
            name,
            model_id,
            cache_dir,
            max_tokens,
            device_policy,
            CandlePrecision::F32,
            CandlePoolingPolicy::Mean,
        )
    }

    pub fn from_model_with_options(
        name: impl Into<String>,
        model_id: impl Into<String>,
        cache_dir: PathBuf,
        max_tokens: usize,
        device_policy: CandleDevicePolicy,
        precision: CandlePrecision,
        pooling: CandlePoolingPolicy,
    ) -> Result<Self> {
        let model_id = model_id.into();
        let files = fetch_files(&cache_dir, &model_id)?;
        Self::from_files(CandleFileSpec {
            name: name.into(),
            model_id,
            cache_dir,
            config: files.config,
            tokenizer: files.tokenizer,
            weights: files.weights,
            max_tokens,
            device_policy,
            precision,
            pooling,
            norm_policy: NormPolicy::unit(),
            expected_dim: None,
            expected_weights_sha256: None,
            contract_paths: Vec::new(),
        })
    }

    pub fn from_files(spec: CandleFileSpec) -> Result<Self> {
        ensure_file("config", &spec.config)?;
        ensure_file("tokenizer", &spec.tokenizer)?;
        ensure_file("weights", &spec.weights)?;
        let required_paths = vec![
            spec.weights.clone(),
            spec.tokenizer.clone(),
            spec.config.clone(),
        ];
        let contract_paths = if spec.contract_paths.is_empty() {
            required_paths.clone()
        } else {
            spec.contract_paths.clone()
        };
        for path in &contract_paths {
            ensure_file("contract artifact", path)?;
        }
        let weights_sha256 = hash_files(&contract_paths)?;
        if let Some(expected) = spec.expected_weights_sha256
            && weights_sha256 != expected
        {
            return Err(CalyxError::lens_frozen_violation(format!(
                "candle artifact hash drift for {}",
                spec.model_id
            )));
        }
        let config = read_config(&spec.config, spec.device_policy, spec.precision)?;
        let dim = u32::try_from(config.hidden_size).map_err(|_| {
            CalyxError::lens_dim_mismatch(format!(
                "candle hidden size {} exceeds u32",
                config.hidden_size
            ))
        })?;
        if let Some(expected) = spec.expected_dim
            && dim != expected
        {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "candle hidden size {dim} != declared {expected}"
            )));
        }
        let tokenizer = read_tokenizer(&spec.tokenizer, spec.max_tokens)?;
        let model = read_model(&spec.weights, &config, spec.device_policy, spec.precision)?;
        let (finite_replay_model, finite_replay_precision) =
            if needs_f32_finite_replay(spec.device_policy, spec.precision) {
                (
                    Some(Mutex::new(read_model(
                        &spec.weights,
                        &config,
                        spec.device_policy,
                        CandlePrecision::F32,
                    )?)),
                    Some(CandlePrecision::F32),
                )
            } else {
                (None, None)
            };
        let files = CandleModelFiles {
            cache_dir: spec.cache_dir,
            model_id: spec.model_id,
            config: spec.config,
            tokenizer: spec.tokenizer,
            weights: spec.weights,
            contract_paths,
        };
        let max_tokens_text = spec.max_tokens.to_string();
        let norm_text = format!("{:?}", spec.norm_policy);
        let finite_replay_text = finite_replay_precision
            .map(CandlePrecision::as_str)
            .unwrap_or("none");
        let corpus_hash = sha256_digest(&[
            b"candle-local-bert-v2",
            files.model_id.as_bytes(),
            max_tokens_text.as_bytes(),
            spec.precision.as_str().as_bytes(),
            spec.pooling.as_str().as_bytes(),
            norm_text.as_bytes(),
            finite_replay_text.as_bytes(),
        ]);
        let contract = FrozenLensContract::new(
            spec.name,
            weights_sha256,
            corpus_hash,
            SlotShape::Dense(dim),
            Modality::Text,
            LensDType::F32,
            spec.norm_policy,
        );
        let id = contract.lens_id();
        Ok(Self {
            id,
            dim,
            contract,
            files,
            device_policy: spec.device_policy,
            precision: spec.precision,
            pooling: spec.pooling,
            max_tokens: spec.max_tokens,
            tokenizer,
            model: Mutex::new(model),
            finite_replay_model,
            finite_replay_precision,
        })
    }

    pub fn from_lens_spec(spec: &LensSpec) -> Result<Self> {
        let LensRuntime::CandleLocal {
            model_id,
            files,
            dtype,
            pooling,
        } = &spec.runtime
        else {
            return Err(config_invalid("LensSpec runtime is not candle_local"));
        };
        let [weights, tokenizer, config, ..] = files.as_slice() else {
            return Err(config_invalid(
                "LensRuntime::CandleLocal requires weights, tokenizer, and config paths",
            ));
        };
        let SlotShape::Dense(dim) = spec.output else {
            return Err(CalyxError::lens_dim_mismatch(
                "candle runtime requires dense output shape",
            ));
        };
        let cache_dir = weights
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::from_files(CandleFileSpec {
            name: spec.name.clone(),
            model_id: model_id.clone(),
            cache_dir,
            config: config.clone(),
            tokenizer: tokenizer.clone(),
            weights: weights.clone(),
            max_tokens: DEFAULT_MAX_TOKENS,
            device_policy: CandleDevicePolicy::CudaFailLoud { ordinal: 0 },
            precision: CandlePrecision::parse(dtype)?,
            pooling: CandlePoolingPolicy::parse(pooling)?,
            norm_policy: spec.norm_policy,
            expected_dim: Some(dim),
            expected_weights_sha256: Some(spec.weights_sha256),
            contract_paths: files.clone(),
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

    pub const fn precision(&self) -> CandlePrecision {
        self.precision
    }

    pub const fn pooling(&self) -> CandlePoolingPolicy {
        self.pooling
    }

    pub const fn max_tokens(&self) -> usize {
        self.max_tokens
    }

    pub const fn finite_replay_precision(&self) -> Option<CandlePrecision> {
        self.finite_replay_precision
    }

    pub fn lens_spec(&self) -> LensSpec {
        LensSpec {
            name: self.contract.name().to_string(),
            runtime: LensRuntime::CandleLocal {
                model_id: self.files.model_id.clone(),
                files: self.files.artifact_paths(),
                dtype: self.precision.as_str().to_string(),
                pooling: self.pooling.as_str().to_string(),
            },
            output: self.contract.shape(),
            modality: self.contract.modality(),
            weights_sha256: self.contract.weights_sha256(),
            corpus_hash: self.contract.corpus_hash(),
            norm_policy: self.contract.norm_policy(),
            max_batch: None,
            axis: None,
            asymmetry: calyx_core::Asymmetry::None,
            quant_default: calyx_core::QuantPolicy::turboquant_default(),
            truncate_dim: None,
            recall_delta: crate::spec::default_recall_delta(),
            retrieval_only: false,
            excluded_from_dedup: false,
        }
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
        match self.measure_with_model(&self.model, &ids, &mask) {
            Ok(vector) => Ok(vector),
            Err(error)
                if error.code == "CALYX_LENS_NUMERICAL_INVARIANT"
                    && self.finite_replay_model.is_some() =>
            {
                let model = self
                    .finite_replay_model
                    .as_ref()
                    .expect("checked finite replay model presence");
                self.measure_with_model(model, &ids, &mask)
            }
            Err(error) => Err(error),
        }
    }
}

impl CandleLens {
    fn measure_with_model(
        &self,
        model: &Mutex<BertModel>,
        ids: &[u32],
        mask: &[u32],
    ) -> Result<SlotVector> {
        let seq = ids.len();
        if seq == 0 {
            return Err(CalyxError::lens_dim_mismatch(
                "candle tokenizer returned no tokens",
            ));
        }

        let model = model.lock().map_err(|_| {
            CalyxError::lens_unreachable("candle model mutex was poisoned during inference")
        })?;
        let device = model.device.clone();
        let input_ids = Tensor::from_vec(ids.to_vec(), (1, seq), &device).map_err(candle_error)?;
        let token_type_ids =
            Tensor::from_vec(vec![0_u32; seq], (1, seq), &device).map_err(candle_error)?;
        let attention_mask =
            Tensor::from_vec(mask.to_vec(), (1, seq), &device).map_err(candle_error)?;
        let hidden = model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))
            .map_err(candle_error)?;
        let hidden = hidden.to_dtype(DType::F32).map_err(candle_error)?;
        let rows = hidden.to_vec3::<f32>().map_err(candle_error)?;
        let first = rows.first().ok_or_else(|| {
            CalyxError::lens_dim_mismatch("candle model returned empty batch output")
        })?;
        let mut data = pool_tokens(first, mask, self.dim as usize, self.pooling)?;
        apply_norm(self.contract.norm_policy(), &mut data)?;
        let vector = SlotVector::Dense {
            dim: self.dim,
            data,
        };
        self.contract.verify_vector(self.id, &vector)?;
        Ok(vector)
    }
}

#[cfg(test)]
mod tests;
