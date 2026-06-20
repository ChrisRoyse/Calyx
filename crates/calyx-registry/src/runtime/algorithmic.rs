use std::collections::BTreeMap;

use calyx_core::{
    Input, Lens, LensId, Modality, Result, SlotShape, SlotVector, SparseEntry, content_address,
};

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::lens::ensure_input_modality;

mod gdelt;

const BYTE_FEATURE_DIM: u32 = 16;
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// Deterministic, data-local feature encoders with no model weights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlgorithmicEncoder {
    /// Byte and character-class features for text/code/structured inputs.
    ByteFeatures,
    /// Single scalar summary.
    Scalar,
    /// Hash-selected one-hot feature vector.
    OneHot { buckets: u32 },
    /// Small AST/code-style feature vector.
    AstStyle,
    /// Hashed whitespace terms in a sparse ambient space.
    SparseKeywords { dim: u32 },
    /// Hashed whitespace terms as per-token vectors for MaxSim.
    TokenHash { token_dim: u32 },
    /// Dense CAMEO/event-code features from GDELT text rows.
    GdeltCameo,
    /// Sparse actor/country/geography entity features from GDELT text rows.
    GdeltActorGeo { dim: u32 },
}

impl AlgorithmicEncoder {
    /// Returns the primary output dimension.
    pub const fn dim(self) -> u32 {
        match self {
            Self::ByteFeatures => BYTE_FEATURE_DIM,
            Self::Scalar => 1,
            Self::OneHot { buckets } => {
                if buckets == 0 {
                    1
                } else {
                    buckets
                }
            }
            Self::AstStyle => 8,
            Self::SparseKeywords { dim } => {
                if dim == 0 {
                    1
                } else {
                    dim
                }
            }
            Self::TokenHash { token_dim } => {
                if token_dim == 0 {
                    1
                } else {
                    token_dim
                }
            }
            Self::GdeltCameo => 16,
            Self::GdeltActorGeo { dim } => {
                if dim == 0 {
                    1
                } else {
                    dim
                }
            }
        }
    }

    pub const fn shape(self) -> SlotShape {
        match self {
            Self::SparseKeywords { dim } | Self::GdeltActorGeo { dim } => {
                SlotShape::Sparse(if dim == 0 { 1 } else { dim })
            }
            Self::TokenHash { token_dim } => SlotShape::Multi {
                token_dim: if token_dim == 0 { 1 } else { token_dim },
            },
            _ => SlotShape::Dense(self.dim()),
        }
    }
}

/// A frozen algorithmic lens.
#[derive(Clone, Debug)]
pub struct AlgorithmicLens {
    id: LensId,
    modality: Modality,
    encoder: AlgorithmicEncoder,
    contract: FrozenLensContract,
}

impl AlgorithmicLens {
    /// Creates an algorithmic byte-feature lens.
    pub fn byte_features(name: impl Into<String>, modality: Modality) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::ByteFeatures)
    }

    pub fn scalar(name: impl Into<String>, modality: Modality) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::Scalar)
    }

    pub fn one_hot(name: impl Into<String>, modality: Modality, buckets: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::OneHot { buckets })
    }

    pub fn ast_style(name: impl Into<String>, modality: Modality) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::AstStyle)
    }

    pub fn sparse_keywords(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::SparseKeywords { dim })
    }

    pub fn token_hash(name: impl Into<String>, modality: Modality, token_dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::TokenHash { token_dim })
    }

    pub fn gdelt_cameo(name: impl Into<String>, modality: Modality) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::GdeltCameo)
    }

    pub fn gdelt_actor_geo(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::GdeltActorGeo { dim })
    }

    /// Creates an algorithmic lens from an encoder.
    pub fn new(name: impl Into<String>, modality: Modality, encoder: AlgorithmicEncoder) -> Self {
        let name = name.into();
        let contract = algorithmic_contract(&name, modality, encoder);
        let id = contract.lens_id();
        Self {
            id,
            modality,
            encoder,
            contract,
        }
    }

    /// Returns the frozen contract that produced this lens id.
    pub fn contract(&self) -> &FrozenLensContract {
        &self.contract
    }
}

impl Lens for AlgorithmicLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        self.encoder.shape()
    }

    fn modality(&self) -> Modality {
        self.modality
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        ensure_input_modality(self, input)?;
        Ok(match self.encoder {
            AlgorithmicEncoder::ByteFeatures => SlotVector::Dense {
                dim: self.encoder.dim(),
                data: byte_features(&input.bytes),
            },
            AlgorithmicEncoder::Scalar => SlotVector::Dense {
                dim: self.encoder.dim(),
                data: scalar_features(&input.bytes),
            },
            AlgorithmicEncoder::OneHot { buckets } => SlotVector::Dense {
                dim: self.encoder.dim(),
                data: one_hot_features(&input.bytes, buckets),
            },
            AlgorithmicEncoder::AstStyle => SlotVector::Dense {
                dim: self.encoder.dim(),
                data: ast_style_features(&input.bytes),
            },
            AlgorithmicEncoder::SparseKeywords { dim } => sparse_keywords(&input.bytes, dim)?,
            AlgorithmicEncoder::TokenHash { token_dim } => token_hash(&input.bytes, token_dim)?,
            AlgorithmicEncoder::GdeltCameo => SlotVector::Dense {
                dim: self.encoder.dim(),
                data: gdelt::cameo_features(&input.bytes),
            },
            AlgorithmicEncoder::GdeltActorGeo { dim } => gdelt::actor_geo(&input.bytes, dim)?,
        })
    }
}

fn algorithmic_contract(
    name: &str,
    modality: Modality,
    encoder: AlgorithmicEncoder,
) -> FrozenLensContract {
    if encoder == AlgorithmicEncoder::ByteFeatures {
        return FrozenLensContract::algorithmic_byte_features(name, modality);
    }
    let encoder_text = format!("{encoder:?}:{}", encoder.dim());
    FrozenLensContract::new(
        name,
        sha256_digest(&[b"algorithmic-runtime-v2", encoder_text.as_bytes()]),
        sha256_digest(&[b"algorithmic-data-oblivious"]),
        encoder.shape(),
        modality,
        LensDType::F32,
        NormPolicy::None,
    )
}

fn byte_features(bytes: &[u8]) -> Vec<f32> {
    let mut out = vec![0.0_f32; BYTE_FEATURE_DIM as usize];
    if bytes.is_empty() {
        out[0] = 1.0;
        return out;
    }

    let mut ascii = 0_u32;
    let mut whitespace = 0_u32;
    let mut alphabetic = 0_u32;
    let mut digits = 0_u32;
    let mut punctuation = 0_u32;
    let mut uppercase = 0_u32;
    let mut lowercase = 0_u32;
    let mut control = 0_u32;
    let mut nul = 0_u32;
    let mut path = 0_u32;
    let mut brackets = 0_u32;
    let mut newline = 0_u32;
    let mut byte_sum = 0_u64;
    let mut hash = FNV_OFFSET;

    for &byte in bytes {
        byte_sum += u64::from(byte);
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
        ascii += byte.is_ascii() as u32;
        whitespace += byte.is_ascii_whitespace() as u32;
        alphabetic += byte.is_ascii_alphabetic() as u32;
        digits += byte.is_ascii_digit() as u32;
        punctuation += byte.is_ascii_punctuation() as u32;
        uppercase += byte.is_ascii_uppercase() as u32;
        lowercase += byte.is_ascii_lowercase() as u32;
        control += byte.is_ascii_control() as u32;
        nul += (byte == 0) as u32;
        path += matches!(byte, b'/' | b'\\') as u32;
        brackets += matches!(byte, b'{' | b'}' | b'(' | b')' | b'[' | b']') as u32;
        newline += matches!(byte, b'\n' | b'\r') as u32;
    }

    let len = bytes.len().min(u32::MAX as usize) as f32;
    let inv_len = 1.0 / len.max(1.0);
    out[0] = len.log2().max(0.0) / 32.0;
    out[1] = ascii as f32 * inv_len;
    out[2] = whitespace as f32 * inv_len;
    out[3] = alphabetic as f32 * inv_len;
    out[4] = digits as f32 * inv_len;
    out[5] = punctuation as f32 * inv_len;
    out[6] = uppercase as f32 * inv_len;
    out[7] = lowercase as f32 * inv_len;
    out[8] = control as f32 * inv_len;
    out[9] = nul as f32 * inv_len;
    out[10] = path as f32 * inv_len;
    out[11] = brackets as f32 * inv_len;
    out[12] = newline as f32 * inv_len;
    out[13] = byte_sum as f32 / (len * 255.0);
    out[14] = hash_part((hash & 0xffff_ffff) as u32);
    out[15] = hash_part((hash >> 32) as u32);
    out
}

fn hash_part(value: u32) -> f32 {
    (value as f32 / u32::MAX as f32) * 2.0 - 1.0
}

fn scalar_features(bytes: &[u8]) -> Vec<f32> {
    if bytes.is_empty() {
        return vec![0.0];
    }
    let mean = bytes.iter().map(|byte| f32::from(*byte)).sum::<f32>() / bytes.len() as f32;
    vec![(mean - 80.0) / 80.0]
}

fn one_hot_features(bytes: &[u8], buckets: u32) -> Vec<f32> {
    let buckets = buckets.max(1);
    let mut out = vec![0.0; buckets as usize];
    let hash = bytes.iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    });
    out[(hash % u64::from(buckets)) as usize] = 1.0;
    out
}

fn ast_style_features(bytes: &[u8]) -> Vec<f32> {
    let text = String::from_utf8_lossy(bytes);
    let len = bytes.len().max(1) as f32;
    let count = |needle: &str| text.matches(needle).count() as f32 / len;
    vec![
        count("fn"),
        count("let"),
        count("struct"),
        count("impl"),
        bytes.iter().filter(|b| matches!(b, b'{' | b'}')).count() as f32 / len,
        bytes.iter().filter(|b| **b == b';').count() as f32 / len,
        bytes.iter().filter(|b| **b == b'(').count() as f32 / len,
        bytes.iter().filter(|b| **b == b'\n').count() as f32 / len,
    ]
}

fn sparse_keywords(bytes: &[u8], dim: u32) -> Result<SlotVector> {
    let dim = dim.max(1);
    let mut counts = BTreeMap::<u32, f32>::new();
    for term in String::from_utf8_lossy(bytes).split_whitespace() {
        let digest = content_address([term.as_bytes()]);
        let hash = u32::from_be_bytes(digest[..4].try_into().expect("content hash has bytes"));
        *counts.entry(hash % dim).or_default() += 1.0;
    }
    let total = counts.values().sum::<f32>().max(1.0);
    Ok(SlotVector::Sparse {
        dim,
        entries: counts
            .into_iter()
            .map(|(idx, val)| SparseEntry {
                idx,
                val: val / total,
            })
            .collect(),
    })
}

fn token_hash(bytes: &[u8], token_dim: u32) -> Result<SlotVector> {
    let token_dim = token_dim.max(1);
    let mut tokens = String::from_utf8_lossy(bytes)
        .split_whitespace()
        .take(32)
        .map(|term| token_vector(term.as_bytes(), token_dim))
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        tokens.push(token_vector(bytes, token_dim));
    }
    Ok(SlotVector::Multi { token_dim, tokens })
}

fn token_vector(seed: &[u8], dim: u32) -> Vec<f32> {
    let mut out = Vec::with_capacity(dim as usize);
    let mut counter = 0_u32;
    while out.len() < dim as usize {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"calyx-algorithmic-token-hash-v1");
        hasher.update(seed);
        hasher.update(&counter.to_be_bytes());
        for chunk in hasher.finalize().as_bytes().chunks_exact(4) {
            let raw = u32::from_be_bytes(chunk.try_into().expect("blake3 chunk is 4 bytes"));
            let unit = (raw as f32 / u32::MAX as f32) * 2.0 - 1.0;
            out.push(unit);
            if out.len() == dim as usize {
                break;
            }
        }
        counter = counter.saturating_add(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_features_are_bit_deterministic() {
        let lens = AlgorithmicLens::byte_features("byte-fsv", Modality::Text);
        let input = Input::new(Modality::Text, b"Calyx PH17: 2+2=4\n".to_vec());

        let first = lens.measure(&input).unwrap();
        let second = lens.measure(&input).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn empty_input_emits_real_dense_vector() {
        let lens = AlgorithmicLens::byte_features("byte-empty", Modality::Text);
        let input = Input::new(Modality::Text, Vec::new());
        let vector = lens.measure(&input).unwrap();
        let bytes = serde_json::to_vec(&vector).unwrap();

        println!(
            "ALGORITHMIC_EMPTY_BYTES={}",
            String::from_utf8_lossy(&bytes)
        );
        assert_eq!(
            vector,
            SlotVector::Dense {
                dim: BYTE_FEATURE_DIM,
                data: {
                    let mut data = vec![0.0; BYTE_FEATURE_DIM as usize];
                    data[0] = 1.0;
                    data
                }
            }
        );
    }

    #[test]
    fn scalar_feature_is_centered_for_cosine_assay() {
        let lens = AlgorithmicLens::scalar("scalar-fsv", Modality::Structured);
        let low = Input::new(Modality::Structured, b"!!!!!!!!!!!!!!!!".to_vec());
        let high = Input::new(Modality::Structured, b"zzzzzzzzzzzzzzzz".to_vec());

        let low = lens.measure(&low).unwrap();
        let high = lens.measure(&high).unwrap();

        assert!(matches!(low, SlotVector::Dense { data, .. } if data[0] < 0.0));
        assert!(matches!(high, SlotVector::Dense { data, .. } if data[0] > 0.0));
    }

    #[test]
    fn algorithmic_fsv_determinism_probe() {
        let lens = AlgorithmicLens::byte_features("byte-fsv", Modality::Text);
        let input = Input::new(Modality::Text, b"Calyx registry manual FSV".to_vec());
        let first = lens.measure(&input).unwrap();
        let second = lens.measure(&input).unwrap();
        let first_bytes = serde_json::to_vec(&first).unwrap();
        let second_bytes = serde_json::to_vec(&second).unwrap();

        println!("ALGORITHMIC_FSV_DIGEST={}", digest_hex(&first_bytes));
        println!(
            "ALGORITHMIC_FSV_BYTES={}",
            String::from_utf8_lossy(&first_bytes)
        );
        assert_eq!(first_bytes, second_bytes);
    }

    fn digest_hex(bytes: &[u8]) -> String {
        calyx_core::content_address([bytes])
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
