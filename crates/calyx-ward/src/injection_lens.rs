//! RoBERTa prompt-injection guard lens for Ward's runtime defense (#697).
//!
//! The fine-tuned `RobertaForSequenceClassification` injection guard (#562,
//! `model_comb`, 2 labels: 0=benign, 1=injection) is exported to ONNX and run
//! here through the same pinned `ort` CUDA session used by [`super::style_lens`].
//! The lens emits a `benign_score = softmax(logits)[benign]` per input; Ward's
//! conformal `calibrate_slot` turns that into a block threshold `tau` (block iff
//! `benign_score < tau`), gated on BOTH injection block-rate AND benign FRR.
//!
//! This is the production seam: the classifier now scores inside the Rust Ward
//! runtime, not only in the offline Python validator, so injections are blocked
//! in-process. The CUDA execution provider is fail-loud — a missing/!working GPU
//! errors out rather than silently falling back to CPU.

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use calyx_core::{
    CalyxError, Input, Lens, LensId, Modality, Result as CalyxResult, SlotShape, SlotVector,
};
use ort::ep::{self, ExecutionProviderDispatch};
use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::{Tensor, TensorElementType, ValueType};
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

use crate::error::WardError;

pub const DEFAULT_INJECTION_MODEL_PATH: &str =
    "/home/croyse/calyx/models/injection-guard/model.onnx";
pub const DEFAULT_INJECTION_TOKENIZER_PATH: &str =
    "/home/croyse/calyx/models/injection-guard/tokenizer.json";
/// RoBERTa positional embeddings cap usable tokens at 512 (514 incl. specials).
pub const INJECTION_MAX_TOKENS: usize = 512;
/// `RobertaForSequenceClassification` injection head: 2 logits.
pub const INJECTION_LABELS: usize = 2;
const BENIGN_LABEL: usize = 0;
const INJECTION_LABEL: usize = 1;
const INJECTION_LENS_NAME: &str = "injection-guard-v1";
const INJECTION_SOURCE_REPO: &str = "calyx/injection_guard#562-model_comb";
const INJECTION_SOURCE_REVISION: &str = "roberta-base/safe-guard+deepset+jackhhao/8ep";
const OUTPUT_SHAPE: &[u8] = b"dense:f32:text:injection_benign_score:1";

/// ONNX execution-provider policy for the injection guard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InjectionProviderPolicy {
    CudaFailLoud,
    CpuExplicit,
}

impl InjectionProviderPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CudaFailLoud => "cuda:0,error_on_failure,no_cpu_fallback",
            Self::CpuExplicit => "cpu_explicit,no_cuda",
        }
    }
}

/// Backend seam: production uses the pinned ONNX session; tests inject scores.
pub trait InjectionScoreBackend: Send + Sync {
    /// Probability the text is BENIGN (`softmax(logits)[benign]`), in `[0, 1]`.
    fn benign_score(&self, text: &str) -> Result<f32, WardError>;

    fn input_names(&self) -> Vec<String> {
        Vec::new()
    }

    fn output_names(&self) -> Vec<String> {
        Vec::new()
    }

    fn provider_policy(&self) -> &'static str {
        "test_backend"
    }
}

/// Frozen prompt-injection guard lens. Runtime state is ORT + tokenizer handles.
pub struct InjectionLens {
    model_path: PathBuf,
    tokenizer_path: PathBuf,
    lens_id: LensId,
    backend: Box<dyn InjectionScoreBackend>,
}

impl fmt::Debug for InjectionLens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InjectionLens")
            .field("model_path", &self.model_path)
            .field("tokenizer_path", &self.tokenizer_path)
            .field("lens_id", &self.lens_id)
            .field("provider_policy", &self.provider_policy())
            .finish()
    }
}

impl InjectionLens {
    pub fn new(model_path: &Path) -> Result<Self, WardError> {
        Self::new_with_provider_policy(model_path, InjectionProviderPolicy::CudaFailLoud)
    }

    pub fn new_cpu_explicit(model_path: &Path) -> Result<Self, WardError> {
        Self::new_with_provider_policy(model_path, InjectionProviderPolicy::CpuExplicit)
    }

    pub fn new_with_provider_policy(
        model_path: &Path,
        policy: InjectionProviderPolicy,
    ) -> Result<Self, WardError> {
        let tokenizer_path = model_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("tokenizer.json");
        Self::new_with_tokenizer_and_provider_policy(model_path, &tokenizer_path, policy)
    }

    pub fn new_with_tokenizer_and_provider_policy(
        model_path: &Path,
        tokenizer_path: &Path,
        policy: InjectionProviderPolicy,
    ) -> Result<Self, WardError> {
        // ONNX stores large weights in a `<model>.data` external-data sidecar;
        // include it (when present) so the lens identity pins the actual weights,
        // not just the tiny graph file.
        let external_data = external_data_path(model_path);
        let mut hash_paths: Vec<&Path> = vec![model_path, tokenizer_path];
        if external_data.is_file() {
            hash_paths.push(external_data.as_path());
        }
        let weights_hash = sha256_files(&hash_paths)?;
        let backend = OnnxInjectionBackend::new(model_path, tokenizer_path, policy)?;
        Self::from_backend(
            model_path.to_path_buf(),
            tokenizer_path.to_path_buf(),
            weights_hash,
            backend,
        )
    }

    pub fn from_backend<B>(
        model_path: PathBuf,
        tokenizer_path: PathBuf,
        weights_sha256: [u8; 32],
        backend: B,
    ) -> Result<Self, WardError>
    where
        B: InjectionScoreBackend + 'static,
    {
        let corpus_hash = hash_parts(&[
            INJECTION_SOURCE_REPO.as_bytes(),
            INJECTION_SOURCE_REVISION.as_bytes(),
            b"input_ids",
            b"attention_mask",
            b"logits",
            b"softmax_benign",
        ]);
        let lens_id = LensId::from_parts(
            INJECTION_LENS_NAME,
            &weights_sha256,
            &corpus_hash,
            OUTPUT_SHAPE,
        );
        Ok(Self {
            model_path,
            tokenizer_path,
            lens_id,
            backend: Box::new(backend),
        })
    }

    /// `P(benign)` for `text`, in `[0, 1]`. Higher = more benign; Ward blocks
    /// when this is below the calibrated `tau`.
    pub fn benign_score(&self, text: &str) -> Result<f32, WardError> {
        if text.trim().is_empty() {
            return Err(WardError::InvalidInput {
                reason: "empty injection-guard text".to_string(),
            });
        }
        let score = self.backend.benign_score(text)?;
        if !score.is_finite() || !(0.0..=1.0).contains(&score) {
            return Err(WardError::InvalidInput {
                reason: format!("injection benign_score {score} outside [0,1]"),
            });
        }
        Ok(score)
    }

    /// `P(injection) = 1 - P(benign)`.
    pub fn injection_prob(&self, text: &str) -> Result<f32, WardError> {
        Ok(1.0 - self.benign_score(text)?)
    }

    pub fn benign_score_batch(&self, texts: &[&str]) -> Result<Vec<f32>, WardError> {
        texts.iter().map(|text| self.benign_score(text)).collect()
    }

    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    pub fn tokenizer_path(&self) -> &Path {
        &self.tokenizer_path
    }

    pub fn provider_policy(&self) -> &'static str {
        self.backend.provider_policy()
    }

    pub fn input_names(&self) -> Vec<String> {
        self.backend.input_names()
    }

    pub fn output_names(&self) -> Vec<String> {
        self.backend.output_names()
    }
}

impl Lens for InjectionLens {
    fn id(&self) -> LensId {
        self.lens_id
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(1)
    }

    fn modality(&self) -> Modality {
        Modality::Text
    }

    fn measure(&self, input: &Input) -> CalyxResult<SlotVector> {
        if input.modality != Modality::Text {
            return Err(ward_as_calyx(WardError::InvalidInput {
                reason: format!("injection lens expects text, got {:?}", input.modality),
            }));
        }
        let text = std::str::from_utf8(&input.bytes).map_err(|err| {
            ward_as_calyx(WardError::InvalidInput {
                reason: format!("injection Input bytes must be UTF-8: {err}"),
            })
        })?;
        let score = self.benign_score(text).map_err(ward_as_calyx)?;
        Ok(SlotVector::Dense {
            dim: 1,
            data: vec![score],
        })
    }
}

struct OnnxInjectionBackend {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    input_ids_name: String,
    attention_mask_name: String,
    output_name: String,
    input_names: Vec<String>,
    output_names: Vec<String>,
    policy: InjectionProviderPolicy,
}

impl OnnxInjectionBackend {
    fn new(
        model_path: &Path,
        tokenizer_path: &Path,
        policy: InjectionProviderPolicy,
    ) -> Result<Self, WardError> {
        let tokenizer =
            Tokenizer::from_file(tokenizer_path).map_err(|_| WardError::ModelNotFound {
                path: tokenizer_path.to_path_buf(),
            })?;
        let session = build_session(model_path, policy)?;
        let input_names = session
            .inputs()
            .iter()
            .map(|input| input.name().to_string())
            .collect::<Vec<_>>();
        let output_names = session
            .outputs()
            .iter()
            .map(|output| output.name().to_string())
            .collect::<Vec<_>>();
        let input_ids_name = choose_name(&input_names, "input_ids", "input")?;
        let attention_mask_name = choose_name(&input_names, "attention_mask", "input")?;
        let output_name = choose_name(&output_names, "logits", "output")?;
        assert_logits_shape(&session, &output_name)?;
        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            input_ids_name,
            attention_mask_name,
            output_name,
            input_names,
            output_names,
            policy,
        })
    }

    fn tokenize(&self, text: &str) -> Result<(Vec<i64>, Vec<i64>), WardError> {
        let encoding = self.tokenizer.encode(text, true).map_err(runtime_error)?;
        let len = encoding.get_ids().len().min(INJECTION_MAX_TOKENS);
        if len == 0 {
            return Err(WardError::InvalidInput {
                reason: "injection tokenizer emitted no tokens".to_string(),
            });
        }
        let ids = encoding
            .get_ids()
            .iter()
            .take(len)
            .map(|value| i64::from(*value))
            .collect::<Vec<_>>();
        let attention = encoding
            .get_attention_mask()
            .iter()
            .take(len)
            .map(|value| i64::from(*value))
            .collect::<Vec<_>>();
        Ok((ids, attention))
    }
}

impl InjectionScoreBackend for OnnxInjectionBackend {
    fn benign_score(&self, text: &str) -> Result<f32, WardError> {
        let (ids, attention) = self.tokenize(text)?;
        let seq_len = ids.len();
        let ids_tensor = Tensor::from_array(([1usize, seq_len], ids)).map_err(runtime_error)?;
        let mask_tensor =
            Tensor::from_array(([1usize, seq_len], attention)).map_err(runtime_error)?;
        let mut session = self.session.lock().map_err(|_| WardError::Runtime {
            reason: "injection lens ORT session mutex poisoned".to_string(),
        })?;
        let outputs = session
            .run(ort::inputs! {
                self.input_ids_name.as_str() => ids_tensor,
                self.attention_mask_name.as_str() => mask_tensor
            })
            .map_err(runtime_error)?;
        let output = outputs
            .get(&self.output_name)
            .ok_or_else(|| WardError::Runtime {
                reason: format!("ONNX output {} missing", self.output_name),
            })?;
        let (_, data) = output.try_extract_tensor::<f32>().map_err(runtime_error)?;
        if data.len() != INJECTION_LABELS {
            return Err(WardError::ModelDimMismatch {
                expected: INJECTION_LABELS,
                actual: data.len(),
            });
        }
        softmax_benign(data[BENIGN_LABEL], data[INJECTION_LABEL])
    }

    fn input_names(&self) -> Vec<String> {
        self.input_names.clone()
    }

    fn output_names(&self) -> Vec<String> {
        self.output_names.clone()
    }

    fn provider_policy(&self) -> &'static str {
        self.policy.as_str()
    }
}

/// Numerically-stable 2-class softmax, returning `P(benign)`.
fn softmax_benign(benign_logit: f32, injection_logit: f32) -> Result<f32, WardError> {
    if !benign_logit.is_finite() || !injection_logit.is_finite() {
        return Err(WardError::InvalidInput {
            reason: "injection logits contain NaN or Inf".to_string(),
        });
    }
    let max = benign_logit.max(injection_logit);
    let benign_exp = (benign_logit - max).exp();
    let injection_exp = (injection_logit - max).exp();
    let denom = benign_exp + injection_exp;
    if denom <= f32::EPSILON {
        return Err(WardError::InvalidInput {
            reason: "injection softmax denominator underflow".to_string(),
        });
    }
    Ok(benign_exp / denom)
}

fn build_session(model_path: &Path, policy: InjectionProviderPolicy) -> Result<Session, WardError> {
    if !model_path.exists() {
        return Err(WardError::ModelNotFound {
            path: model_path.to_path_buf(),
        });
    }
    let builder = Session::builder()
        .map_err(runtime_error)?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(runtime_error)?;
    let mut builder = builder
        .with_execution_providers(execution_providers(policy))
        .map_err(runtime_error)?;
    builder.commit_from_file(model_path).map_err(runtime_error)
}

fn execution_providers(policy: InjectionProviderPolicy) -> Vec<ExecutionProviderDispatch> {
    match policy {
        InjectionProviderPolicy::CudaFailLoud => vec![
            ep::CUDA::default()
                .with_device_id(0)
                .build()
                .error_on_failure(),
        ],
        InjectionProviderPolicy::CpuExplicit => vec![ep::CPU::default().build()],
    }
}

fn choose_name(names: &[String], preferred: &str, kind: &str) -> Result<String, WardError> {
    names
        .iter()
        .find(|name| name.as_str() == preferred)
        .cloned()
        .ok_or_else(|| WardError::Runtime {
            reason: format!("ONNX session has no {kind} named {preferred}"),
        })
}

/// The injection head must be an f32 tensor whose last static dim is 2.
fn assert_logits_shape(session: &Session, output_name: &str) -> Result<(), WardError> {
    let outlet = session
        .outputs()
        .iter()
        .find(|output| output.name() == output_name)
        .ok_or_else(|| WardError::Runtime {
            reason: format!("ONNX output {output_name} missing from metadata"),
        })?;
    match outlet.dtype() {
        ValueType::Tensor { ty, shape, .. } if *ty == TensorElementType::Float32 => {
            match shape.iter().rev().copied().find(|dim| *dim > 0) {
                Some(dim) if dim as usize == INJECTION_LABELS => Ok(()),
                Some(dim) => Err(WardError::ModelDimMismatch {
                    expected: INJECTION_LABELS,
                    actual: dim as usize,
                }),
                // Fully-dynamic logits dim: validated per-call against the
                // extracted tensor length instead.
                None => Ok(()),
            }
        }
        other => Err(WardError::Runtime {
            reason: format!("ONNX output {output_name} is not f32 tensor: {other:?}"),
        }),
    }
}

/// ONNX external-data sidecar path for `model.onnx` -> `model.onnx.data`.
fn external_data_path(model_path: &Path) -> PathBuf {
    let mut name = model_path.file_name().unwrap_or_default().to_os_string();
    name.push(".data");
    model_path.with_file_name(name)
}

fn sha256_files(paths: &[&Path]) -> Result<[u8; 32], WardError> {
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    for path in paths {
        let mut file = File::open(path).map_err(|_| WardError::ModelNotFound {
            path: (*path).to_path_buf(),
        })?;
        loop {
            let n = file.read(&mut buf).map_err(runtime_error)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }
    Ok(hasher.finalize().into())
}

fn hash_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn runtime_error(error: impl fmt::Display) -> WardError {
    WardError::Runtime {
        reason: error.to_string(),
    }
}

fn ward_as_calyx(error: WardError) -> CalyxError {
    CalyxError {
        code: error.code(),
        message: error.to_string(),
        remediation: "fix Ward injection lens model/input and retry",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubBackend {
        benign: f32,
    }

    impl InjectionScoreBackend for StubBackend {
        fn benign_score(&self, _text: &str) -> Result<f32, WardError> {
            Ok(self.benign)
        }
    }

    fn lens_with(benign: f32) -> InjectionLens {
        InjectionLens::from_backend(
            PathBuf::from("model.onnx"),
            PathBuf::from("tokenizer.json"),
            [7u8; 32],
            StubBackend { benign },
        )
        .expect("lens")
    }

    #[test]
    fn softmax_matches_hand_computed() {
        // logits [2.0, 0.0]: benign = e^2/(e^2+e^0) = 7.389/8.389 = 0.8808.
        let score = softmax_benign(2.0, 0.0).expect("softmax");
        assert!((score - 0.880_797).abs() < 1e-4, "got {score}");
        // Symmetric: equal logits -> 0.5.
        assert!((softmax_benign(1.0, 1.0).expect("eq") - 0.5).abs() < 1e-6);
        // Injection-dominant logits -> low benign score.
        assert!(softmax_benign(-3.0, 3.0).expect("inj") < 0.01);
    }

    #[test]
    fn softmax_rejects_nonfinite() {
        assert_eq!(
            softmax_benign(f32::NAN, 0.0).unwrap_err().code(),
            "CALYX_WARD_INVALID_INPUT"
        );
    }

    #[test]
    fn benign_score_validates_range_and_empty() {
        let lens = lens_with(0.9);
        assert!((lens.benign_score("hello").expect("score") - 0.9).abs() < 1e-6);
        assert!((lens.injection_prob("hello").expect("prob") - 0.1).abs() < 1e-6);
        assert_eq!(
            lens.benign_score("   ").unwrap_err().code(),
            "CALYX_WARD_INVALID_INPUT"
        );
        // Out-of-range backend score is caught.
        let bad = lens_with(1.5);
        assert_eq!(
            bad.benign_score("x").unwrap_err().code(),
            "CALYX_WARD_INVALID_INPUT"
        );
    }

    #[test]
    fn measure_emits_single_dim_score() {
        let lens = lens_with(0.42);
        let input = Input::new(Modality::Text, b"some text".to_vec());
        match lens.measure(&input).expect("measure") {
            SlotVector::Dense { dim, data } => {
                assert_eq!(dim, 1);
                assert_eq!(data.len(), 1);
                assert!((data[0] - 0.42).abs() < 1e-6);
            }
            other => panic!("expected dense, got {other:?}"),
        }
    }
}
