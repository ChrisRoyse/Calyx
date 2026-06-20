use std::path::{Path, PathBuf};
use std::sync::Mutex;

use calyx_core::{CalyxError, Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use ort::session::Session;
use ort::value::ValueType;
use serde_json::Value;
use tokenizers::Tokenizer;

use super::colbert_files::fetch_answerai_colbert_files;
use super::colbert_tokens::multi_rows;
use super::cuda_guard::CudaDropGuard;
use super::custom::batch::{TokenBatch, max_tokens_from_config, session_inputs, token_batches};
use super::fastembed_runtime::execution_providers;
use super::{OnnxModelFiles, OnnxProviderPolicy, config_invalid};
use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::runtime::common::hash_files;
use crate::spec::{LensRuntime, LensSpec, default_recall_delta};

pub const DEFAULT_ANSWERAI_COLBERT_MODEL: &str = "answerdotai/answerai-colbert-small-v1";
pub(in crate::runtime::onnx) const DEFAULT_COLBERT_ONNX: &str = "onnx/model_int8.onnx";

#[derive(Clone, Debug, PartialEq)]
pub struct OnnxColbertFileSpec {
    pub name: String,
    pub model_id: String,
    pub model_file: PathBuf,
    pub tokenizer: PathBuf,
    pub config: PathBuf,
    pub provider_policy: OnnxProviderPolicy,
    pub max_batch: Option<usize>,
    pub expected_shape: Option<SlotShape>,
    pub expected_weights_sha256: Option<[u8; 32]>,
    pub contract_paths: Vec<PathBuf>,
}

pub struct OnnxColbertLens {
    id: LensId,
    token_dim: u32,
    contract: FrozenLensContract,
    files: OnnxModelFiles,
    provider_policy: OnnxProviderPolicy,
    max_batch: Option<usize>,
    runtime: Mutex<OnnxColbertRuntime>,
}

struct OnnxColbertRuntime {
    session: Option<Session>,
    tokenizer: Tokenizer,
    token_dim: u32,
    max_tokens: usize,
}

impl OnnxColbertFileSpec {
    pub fn text(
        name: impl Into<String>,
        model_id: impl Into<String>,
        model_file: impl Into<PathBuf>,
        tokenizer: impl Into<PathBuf>,
        config: impl Into<PathBuf>,
    ) -> Self {
        Self {
            name: name.into(),
            model_id: model_id.into(),
            model_file: model_file.into(),
            tokenizer: tokenizer.into(),
            config: config.into(),
            provider_policy: OnnxProviderPolicy::CudaFailLoud,
            max_batch: None,
            expected_shape: None,
            expected_weights_sha256: None,
            contract_paths: Vec::new(),
        }
    }

    pub fn from_lens_spec(spec: &LensSpec) -> Result<Self> {
        let LensRuntime::OnnxColbert { model_id, files } = &spec.runtime else {
            return Err(config_invalid("LensSpec runtime is not onnx-colbert"));
        };
        let [model_file, tokenizer, config, ..] = files.as_slice() else {
            return Err(config_invalid(
                "LensRuntime::OnnxColbert requires model, tokenizer, and config paths",
            ));
        };
        if spec.max_batch == Some(0) {
            return Err(config_invalid("LensSpec max_batch must be > 0"));
        }
        Ok(Self {
            name: spec.name.clone(),
            model_id: model_id.clone(),
            model_file: model_file.clone(),
            tokenizer: tokenizer.clone(),
            config: config.clone(),
            provider_policy: OnnxProviderPolicy::CudaFailLoud,
            max_batch: spec.max_batch,
            expected_shape: Some(spec.output),
            expected_weights_sha256: Some(spec.weights_sha256),
            contract_paths: files.clone(),
        })
    }

    pub const fn with_provider_policy(mut self, policy: OnnxProviderPolicy) -> Self {
        self.provider_policy = policy;
        self
    }

    pub fn with_contract_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.contract_paths = paths;
        self
    }
}

impl OnnxColbertLens {
    pub fn from_model_id_with_policy(
        name: impl Into<String>,
        model_id: &str,
        cache_dir: PathBuf,
        provider_policy: OnnxProviderPolicy,
    ) -> Result<Self> {
        let model_id = answerai_colbert_model_id(model_id)?;
        let files = fetch_answerai_colbert_files(&cache_dir, &model_id)?;
        let spec = OnnxColbertFileSpec::text(
            name,
            model_id,
            files.model_file.clone(),
            files.tokenizer.clone(),
            files.config.clone(),
        )
        .with_provider_policy(provider_policy)
        .with_contract_paths(files.artifact_paths());
        Self::from_files(spec)
    }

    pub fn from_files(spec: OnnxColbertFileSpec) -> Result<Self> {
        let _ort_dylib = super::dynamic_ort::ensure_dynamic_ort()?;
        ensure_file("model", &spec.model_file)?;
        ensure_file("tokenizer", &spec.tokenizer)?;
        ensure_file("config", &spec.config)?;
        let config = validate_config(&spec.config)?;
        let max_tokens = max_tokens_from_config(&config)?;
        let files = model_files(&spec);
        let weights_sha256 = hash_files(&files.artifact_paths())
            .map_err(|err| config_invalid(format!("read ONNX ColBERT artifacts failed: {err}")))?;
        if let Some(expected) = spec.expected_weights_sha256
            && expected != weights_sha256
        {
            return Err(CalyxError::lens_frozen_violation(format!(
                "ONNX ColBERT artifact hash drift for {}",
                spec.model_id
            )));
        }
        let session = Session::builder()
            .map_err(|err| config_invalid(format!("ONNX session builder failed: {err}")))?
            .with_intra_threads(1)
            .map_err(|err| config_invalid(format!("ONNX intra-thread config failed: {err}")))?
            .with_execution_providers(execution_providers(spec.provider_policy))
            .map_err(|err| config_invalid(format!("ONNX provider config failed: {err}")))?
            .commit_from_file(&spec.model_file)
            .map_err(|err| config_invalid(format!("load ONNX ColBERT model failed: {err}")))?;
        let session = CudaDropGuard::new(session, spec.provider_policy);
        let token_dim = output_token_dim(session.as_ref())?;
        let shape = SlotShape::Multi { token_dim };
        if let Some(expected) = spec.expected_shape
            && expected != shape
        {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "ONNX ColBERT output shape {shape:?} != declared {expected:?}"
            )));
        }
        let tokenizer = Tokenizer::from_file(&spec.tokenizer)
            .map_err(|err| config_invalid(format!("load tokenizer failed: {err}")))?;
        let corpus_hash = sha256_digest(&[
            b"onnx-colbert-token-v1",
            spec.model_id.as_bytes(),
            DEFAULT_COLBERT_ONNX.as_bytes(),
            b"attention-mask-unpooled-finite",
        ]);
        let contract = FrozenLensContract::new(
            spec.name,
            weights_sha256,
            corpus_hash,
            shape,
            Modality::Text,
            LensDType::F32,
            NormPolicy::Finite,
        );
        let runtime = OnnxColbertRuntime {
            session: Some(session.into_inner()),
            tokenizer,
            token_dim,
            max_tokens,
        };
        Ok(Self {
            id: contract.lens_id(),
            token_dim,
            contract,
            files,
            provider_policy: spec.provider_policy,
            max_batch: spec.max_batch,
            runtime: Mutex::new(runtime),
        })
    }

    pub fn from_lens_spec(spec: &LensSpec) -> Result<Self> {
        Self::from_files(OnnxColbertFileSpec::from_lens_spec(spec)?)
    }

    pub fn contract(&self) -> &FrozenLensContract {
        &self.contract
    }

    pub fn files(&self) -> &OnnxModelFiles {
        &self.files
    }

    pub const fn provider_policy(&self) -> &'static str {
        self.provider_policy.as_str()
    }

    pub const fn runtime_name(&self) -> &'static str {
        "onnx-colbert"
    }

    pub fn lens_spec(&self) -> LensSpec {
        LensSpec {
            name: self.contract.name().to_string(),
            runtime: LensRuntime::OnnxColbert {
                model_id: self.files.model_code.clone(),
                files: self.files.artifact_paths(),
            },
            output: self.contract.shape(),
            modality: self.contract.modality(),
            weights_sha256: self.contract.weights_sha256(),
            corpus_hash: self.contract.corpus_hash(),
            norm_policy: self.contract.norm_policy(),
            max_batch: self.max_batch,
            axis: None,
            asymmetry: calyx_core::Asymmetry::None,
            quant_default: calyx_core::QuantPolicy::turboquant_default(),
            truncate_dim: None,
            recall_delta: default_recall_delta(),
            retrieval_only: false,
            excluded_from_dedup: false,
        }
    }
}

impl Lens for OnnxColbertLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Multi {
            token_dim: self.token_dim,
        }
    }

    fn modality(&self) -> Modality {
        Modality::Text
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        let mut batch = self.measure_batch(std::slice::from_ref(input))?;
        batch.pop().ok_or_else(|| {
            CalyxError::lens_dim_mismatch(format!("lens {} returned no ColBERT vector", self.id))
        })
    }

    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| CalyxError::lens_unreachable("ONNX ColBERT mutex was poisoned"))?;
        let chunk_size = self.max_batch.unwrap_or(inputs.len()).max(1);
        if chunk_size >= inputs.len() {
            return runtime.measure_batch(self, inputs, self.contract());
        }
        let mut out = Vec::with_capacity(inputs.len());
        for chunk in inputs.chunks(chunk_size) {
            out.extend(runtime.measure_batch(self, chunk, self.contract())?);
        }
        Ok(out)
    }
}

impl Drop for OnnxColbertLens {
    fn drop(&mut self) {
        if self.provider_policy != OnnxProviderPolicy::CudaFailLoud {
            return;
        }
        if let Ok(runtime) = self.runtime.get_mut()
            && let Some(session) = runtime.session.take()
        {
            std::mem::forget(session);
        }
    }
}

impl OnnxColbertRuntime {
    fn measure_batch(
        &mut self,
        lens: &dyn Lens,
        inputs: &[Input],
        contract: &FrozenLensContract,
    ) -> Result<Vec<SlotVector>> {
        let batches = token_batches(&self.tokenizer, lens, inputs, self.max_tokens)?;
        let mut rows = vec![None; inputs.len()];
        for batch in &batches {
            let vectors = self.run_token_batch(batch)?;
            if vectors.len() != batch.indices.len() {
                return Err(CalyxError::lens_dim_mismatch(format!(
                    "ONNX ColBERT returned {} rows for {} bucketed inputs",
                    vectors.len(),
                    batch.indices.len()
                )));
            }
            for (index, data) in batch.indices.iter().copied().zip(vectors) {
                rows[index] = Some(data);
            }
        }
        rows.into_iter()
            .map(|tokens| {
                let tokens = tokens.ok_or_else(|| {
                    CalyxError::lens_dim_mismatch("ONNX ColBERT omitted a bucketed row")
                })?;
                let vector = SlotVector::Multi {
                    token_dim: self.token_dim,
                    tokens,
                };
                contract.verify_vector(lens.id(), &vector)?;
                Ok(vector)
            })
            .collect()
    }

    fn run_token_batch(&mut self, batch: &TokenBatch) -> Result<Vec<Vec<Vec<f32>>>> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| CalyxError::lens_unreachable("ONNX ColBERT session is unavailable"))?;
        let input_values = session_inputs(session, batch)?;
        let outputs = session
            .run(input_values)
            .map_err(|err| config_invalid(format!("ONNX ColBERT inference failed: {err}")))?;
        let output = output_tensor(&outputs)?;
        let (shape, values) = output.try_extract_tensor::<f32>().map_err(|err| {
            config_invalid(format!("ONNX ColBERT output is not f32 tensor: {err}"))
        })?;
        multi_rows(shape, values, batch, self.token_dim as usize)
    }
}

fn answerai_colbert_model_id(raw: &str) -> Result<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "answerdotai/answerai-colbert-small-v1" | "answerai-colbert-small-v1" => {
            Ok(DEFAULT_ANSWERAI_COLBERT_MODEL.to_string())
        }
        other => Err(CalyxError::lens_unreachable(format!(
            "unsupported onnx-colbert model {other}; expected {DEFAULT_ANSWERAI_COLBERT_MODEL}"
        ))),
    }
}

fn model_files(spec: &OnnxColbertFileSpec) -> OnnxModelFiles {
    let cache_dir = spec
        .model_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    OnnxModelFiles {
        cache_dir,
        model_code: spec.model_id.clone(),
        model_file: spec.model_file.clone(),
        tokenizer: spec.tokenizer.clone(),
        config: spec.config.clone(),
        special_tokens_map: spec.config.clone(),
        tokenizer_config: spec.tokenizer.clone(),
        contract_paths: spec.contract_paths.clone(),
    }
}

fn ensure_file(label: &str, path: &Path) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }
    Err(config_invalid(format!(
        "ONNX ColBERT {label} file {} is missing",
        path.display()
    )))
}

fn validate_config(path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path).map_err(|err| {
        config_invalid(format!(
            "read ONNX ColBERT config {} failed: {err}",
            path.display()
        ))
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        config_invalid(format!(
            "parse ONNX ColBERT config {} failed: {err}",
            path.display()
        ))
    })
}

fn output_token_dim(session: &Session) -> Result<u32> {
    let output = session
        .outputs()
        .iter()
        .find(|out| matches!(out.dtype(), ValueType::Tensor { .. }))
        .ok_or_else(|| config_invalid("ONNX ColBERT model has no tensor outputs"))?;
    let ValueType::Tensor { shape, .. } = output.dtype() else {
        return Err(config_invalid("ONNX ColBERT output is not a tensor"));
    };
    let Some(dim) = shape.last().copied().filter(|dim| *dim > 0) else {
        return Err(config_invalid(format!(
            "ONNX ColBERT output {} has no static token dimension",
            output.name()
        )));
    };
    u32::try_from(dim).map_err(|_| CalyxError::lens_dim_mismatch("ColBERT token dim exceeds u32"))
}

fn output_tensor<'a, 'r>(
    outputs: &'a ort::session::SessionOutputs<'r>,
) -> Result<&'a ort::value::DynValue> {
    for name in [
        "last_hidden_state",
        "token_embeddings",
        "output",
        "sentence_embedding",
    ] {
        if let Some(output) = outputs.get(name) {
            return Ok(output);
        }
    }
    if outputs.len() == 0 {
        return Err(config_invalid("ONNX ColBERT model returned no outputs"));
    }
    Ok(&outputs[0])
}
