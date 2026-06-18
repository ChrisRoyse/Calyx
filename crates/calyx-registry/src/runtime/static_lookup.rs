use std::fs::File;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use memmap2::Mmap;
use tokenizers::{Encoding, Tokenizer, TruncationParams};

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::runtime::common::{DEFAULT_MAX_TOKENS, hash_files, normalize_unit, text_from_input};
use crate::spec::{LensRuntime, LensSpec};

const MAGIC: &[u8; 8] = b"CXLKUP1\0";
const HEADER_LEN: usize = 24;
const DTYPE_I8: u8 = 1;
const DTYPE_F16: u8 = 2;
const DTYPE_F32: u8 = 3;
const UNK_TOKENS: &[&str] = &["[UNK]", "<unk>", "<UNK>"];

#[derive(Debug)]
pub struct StaticLookupLens {
    id: LensId,
    contract: FrozenLensContract,
    files: StaticLookupFiles,
    tokenizer: Tokenizer,
    matrix: StaticLookupMatrix,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticLookupFiles {
    pub embeddings_file: PathBuf,
    pub tokenizer: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StaticLookupFileSpec {
    pub name: String,
    pub embeddings_file: PathBuf,
    pub tokenizer: PathBuf,
    pub dim: Option<u32>,
    pub norm_policy: NormPolicy,
    pub expected_weights_sha256: Option<[u8; 32]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticLookupDType {
    Int8,
    F16,
    F32,
}

impl StaticLookupDType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Int8 => "int8",
            Self::F16 => "f16",
            Self::F32 => "f32",
        }
    }

    const fn width(self) -> usize {
        match self {
            Self::Int8 => 1,
            Self::F16 => 2,
            Self::F32 => 4,
        }
    }
}

#[derive(Debug)]
struct StaticLookupMatrix {
    mmap: Mmap,
    rows: u32,
    dim: u32,
    dtype: StaticLookupDType,
    scale: f32,
}

impl StaticLookupLens {
    pub fn from_files(spec: StaticLookupFileSpec) -> Result<Self> {
        ensure_file("embeddings_file", &spec.embeddings_file)?;
        ensure_file("tokenizer", &spec.tokenizer)?;
        let matrix = StaticLookupMatrix::open(&spec.embeddings_file)?;
        if let Some(expected_dim) = spec.dim
            && matrix.dim != expected_dim
        {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "static lookup matrix dim {} != expected {expected_dim}",
                matrix.dim
            )));
        }
        let tokenizer = read_tokenizer(&spec.tokenizer)?;
        let weights_sha256 = hash_files(&[spec.embeddings_file.clone(), spec.tokenizer.clone()])?;
        if let Some(expected) = spec.expected_weights_sha256
            && weights_sha256 != expected
        {
            return Err(CalyxError::lens_frozen_violation(
                "static lookup matrix/tokenizer hash does not match LensSpec",
            ));
        }
        let dim_text = matrix.dim.to_string();
        let dtype_text = matrix.dtype.as_str();
        let corpus_hash = sha256_digest(&[
            b"static-lookup-model2vec-v1",
            dim_text.as_bytes(),
            dtype_text.as_bytes(),
        ]);
        let contract = FrozenLensContract::new(
            spec.name,
            weights_sha256,
            corpus_hash,
            SlotShape::Dense(matrix.dim),
            Modality::Text,
            LensDType::F32,
            spec.norm_policy,
        );
        let id = contract.lens_id();
        Ok(Self {
            id,
            contract,
            files: StaticLookupFiles {
                embeddings_file: spec.embeddings_file,
                tokenizer: spec.tokenizer,
            },
            tokenizer,
            matrix,
        })
    }

    pub fn from_lens_spec(spec: &LensSpec) -> Result<Self> {
        let LensRuntime::StaticLookup {
            embeddings_file,
            tokenizer,
            dim,
        } = &spec.runtime
        else {
            return Err(config_invalid("LensSpec runtime is not static_lookup"));
        };
        Self::from_files(StaticLookupFileSpec {
            name: spec.name.clone(),
            embeddings_file: embeddings_file.clone(),
            tokenizer: tokenizer.clone(),
            dim: Some(*dim),
            norm_policy: spec.norm_policy,
            expected_weights_sha256: Some(spec.weights_sha256),
        })
    }

    pub fn contract(&self) -> &FrozenLensContract {
        &self.contract
    }

    pub fn files(&self) -> &StaticLookupFiles {
        &self.files
    }

    pub fn dtype(&self) -> StaticLookupDType {
        self.matrix.dtype
    }

    pub fn row_count(&self) -> u32 {
        self.matrix.rows
    }

    pub fn lens_spec(&self) -> LensSpec {
        LensSpec {
            name: self.contract.name().to_string(),
            runtime: LensRuntime::StaticLookup {
                embeddings_file: self.files.embeddings_file.clone(),
                tokenizer: self.files.tokenizer.clone(),
                dim: self.matrix.dim,
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

impl Lens for StaticLookupLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(self.matrix.dim)
    }

    fn modality(&self) -> Modality {
        Modality::Text
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        let text = text_from_input(self, input)?;
        let mut data = if text.trim().is_empty() {
            zero_safe_unit(self.matrix.dim)
        } else {
            let encoding = self
                .tokenizer
                .encode(text, true)
                .map_err(|err| CalyxError::lens_dim_mismatch(format!("tokenize failed: {err}")))?;
            self.pool_encoding(&encoding)?
        };
        apply_norm(self.contract.norm_policy(), &mut data)?;
        let vector = SlotVector::Dense {
            dim: self.matrix.dim,
            data,
        };
        self.contract.verify_vector(self.id, &vector)?;
        Ok(vector)
    }
}

impl StaticLookupLens {
    fn pool_encoding(&self, encoding: &Encoding) -> Result<Vec<f32>> {
        let ids = encoding.get_ids();
        let tokens = encoding.get_tokens();
        let mut out = vec![0.0_f32; self.matrix.dim as usize];
        let mut count = 0_u32;
        for (idx, token_id) in ids.iter().copied().enumerate() {
            if tokens.get(idx).is_some_and(|token| is_unknown_token(token)) {
                continue;
            }
            if self.matrix.add_row(token_id, &mut out)? {
                count += 1;
            }
        }
        if count == 0 {
            return Ok(zero_safe_unit(self.matrix.dim));
        }
        let inv = 1.0 / count as f32;
        for value in &mut out {
            *value *= inv;
        }
        Ok(out)
    }
}

impl StaticLookupMatrix {
    fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|err| {
            CalyxError::lens_unreachable(format!(
                "open static lookup matrix {} failed: {err}",
                path.display()
            ))
        })?;
        let mmap = unsafe {
            Mmap::map(&file).map_err(|err| {
                CalyxError::lens_unreachable(format!(
                    "mmap static lookup matrix {} failed: {err}",
                    path.display()
                ))
            })?
        };
        if mmap.len() < HEADER_LEN || &mmap[..8] != MAGIC {
            return Err(config_invalid(format!(
                "static lookup matrix {} has invalid magic/header",
                path.display()
            )));
        }
        let rows = read_u32(&mmap[8..12]);
        let dim = read_u32(&mmap[12..16]);
        if rows == 0 || dim == 0 {
            return Err(CalyxError::lens_dim_mismatch(
                "static lookup matrix rows and dim must be non-zero",
            ));
        }
        let dtype = match mmap[16] {
            DTYPE_I8 => StaticLookupDType::Int8,
            DTYPE_F16 => StaticLookupDType::F16,
            DTYPE_F32 => StaticLookupDType::F32,
            other => {
                return Err(config_invalid(format!(
                    "unsupported static lookup dtype {other}"
                )));
            }
        };
        let scale = f32::from_le_bytes(mmap[20..24].try_into().expect("header scale"));
        if !scale.is_finite() || scale <= 0.0 {
            return Err(CalyxError::lens_numerical_invariant(
                "static lookup matrix scale must be finite and positive",
            ));
        }
        let body_len = rows as usize * dim as usize * dtype.width();
        let expected = HEADER_LEN
            .checked_add(body_len)
            .ok_or_else(|| CalyxError::lens_dim_mismatch("static lookup matrix size overflow"))?;
        if mmap.len() != expected {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "static lookup matrix byte length {} != expected {expected}",
                mmap.len()
            )));
        }
        Ok(Self {
            mmap,
            rows,
            dim,
            dtype,
            scale,
        })
    }

    fn add_row(&self, row: u32, out: &mut [f32]) -> Result<bool> {
        if row >= self.rows {
            return Ok(false);
        }
        let dim = self.dim as usize;
        if out.len() != dim {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "static lookup output dim {} != matrix dim {dim}",
                out.len()
            )));
        }
        let start = HEADER_LEN + row as usize * dim * self.dtype.width();
        match self.dtype {
            StaticLookupDType::Int8 => {
                for (dst, raw) in out.iter_mut().zip(&self.mmap[start..start + dim]) {
                    *dst += (*raw as i8) as f32 * self.scale;
                }
            }
            StaticLookupDType::F16 => {
                for (idx, dst) in out.iter_mut().enumerate() {
                    let pos = start + idx * 2;
                    let raw = u16::from_le_bytes([self.mmap[pos], self.mmap[pos + 1]]);
                    *dst += f16_to_f32(raw) * self.scale;
                }
            }
            StaticLookupDType::F32 => {
                for (idx, dst) in out.iter_mut().enumerate() {
                    let pos = start + idx * 4;
                    let raw = f32::from_le_bytes(
                        self.mmap[pos..pos + 4].try_into().expect("f32 row bytes"),
                    );
                    *dst += raw * self.scale;
                }
            }
        }
        Ok(true)
    }
}

fn read_tokenizer(path: &Path) -> Result<Tokenizer> {
    let mut tokenizer = Tokenizer::from_file(path)
        .map_err(|err| config_invalid(format!("load static tokenizer failed: {err}")))?;
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: DEFAULT_MAX_TOKENS,
            ..Default::default()
        }))
        .map_err(|err| CalyxError::lens_dim_mismatch(format!("set truncation failed: {err}")))?;
    Ok(tokenizer)
}

fn apply_norm(policy: NormPolicy, data: &mut [f32]) -> Result<()> {
    if data.iter().any(|value| !value.is_finite()) {
        return Err(CalyxError::lens_numerical_invariant(
            "static lookup emitted NaN or Inf",
        ));
    }
    match policy {
        NormPolicy::None | NormPolicy::Finite => Ok(()),
        NormPolicy::L2 { .. } | NormPolicy::Unit { .. } => normalize_unit(data),
        NormPolicy::DeclaredByModel { .. } => Ok(()),
    }
}

fn zero_safe_unit(dim: u32) -> Vec<f32> {
    let mut data = vec![0.0_f32; dim as usize];
    if let Some(first) = data.first_mut() {
        *first = 1.0;
    }
    data
}

fn is_unknown_token(token: &str) -> bool {
    UNK_TOKENS.contains(&token)
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("u32 header bytes"))
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = ((bits & 0x8000) as u32) << 16;
    let exp = (bits >> 10) & 0x1f;
    let frac = (bits & 0x03ff) as u32;
    let value = match exp {
        0 if frac == 0 => sign,
        0 => {
            let mut mant = frac;
            let mut e = -14_i32;
            while mant & 0x0400 == 0 {
                mant <<= 1;
                e -= 1;
            }
            mant &= 0x03ff;
            sign | (((e + 127) as u32) << 23) | (mant << 13)
        }
        0x1f => sign | 0x7f80_0000 | (frac << 13),
        _ => sign | (((exp as i32 - 15 + 127) as u32) << 23) | (frac << 13),
    };
    f32::from_bits(value)
}

fn ensure_file(label: &str, path: &Path) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(config_invalid(format!(
            "static lookup {label} {} is not a file",
            path.display()
        )))
    }
}

fn config_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_LENS_CONFIG_INVALID",
        message: message.into(),
        remediation: "fix static lookup matrix/tokenizer or register a supported lens spec",
    }
}

#[cfg(test)]
mod tests;
