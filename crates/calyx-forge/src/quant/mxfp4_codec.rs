use std::collections::BTreeMap;

use crate::mxfp4::{MXFP4_PACKED_BYTES, MxFp4Block, decode_mxfp4, encode_mxfp4};
use crate::mxfp8::{MXFP8_BLOCK_BYTES, MxFp8Block, decode_mxfp8, encode_mxfp8};
use crate::quant::{QuantLevel, QuantizedVec, Quantizer, SeedId};
use crate::{ForgeError, Result};

const MXFP4_BLOCK_BYTES: usize = MXFP4_PACKED_BYTES + 1;
const ZERO_SEED: SeedId = [0; 32];
const MXFP_REMEDIATION: &str =
    "Use finite vectors, matching dims, Bits4Fp/Bits8Fp bytes, and Assay-safe slots";

#[derive(Clone, Debug)]
pub struct MxFp4Codec {
    dim: usize,
    assay_safe_slots: BTreeMap<String, AssayQuantSafety>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssayQuantSafety {
    pub baseline_bits: f32,
    pub quantized_bits: f32,
    pub cosine: f32,
    pub far_delta: f32,
}

impl AssayQuantSafety {
    pub const MIN_RETAINED_FRACTION: f32 = 0.95;
    pub const MIN_COSINE: f32 = 0.99;
    pub const MAX_FAR_DELTA: f32 = 0.01;

    pub fn passes(&self) -> bool {
        let retained = if self.baseline_bits <= 0.0 {
            self.quantized_bits >= 0.0
        } else {
            self.quantized_bits / self.baseline_bits >= Self::MIN_RETAINED_FRACTION
        };
        retained
            && self.cosine >= Self::MIN_COSINE
            && self.far_delta <= Self::MAX_FAR_DELTA
            && self.values_are_finite()
    }

    fn values_are_finite(&self) -> bool {
        self.baseline_bits.is_finite()
            && self.quantized_bits.is_finite()
            && self.cosine.is_finite()
            && self.far_delta.is_finite()
    }
}

impl MxFp4Codec {
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            assay_safe_slots: BTreeMap::new(),
        }
    }

    pub fn with_assay_safety(
        dim: usize,
        slots: impl IntoIterator<Item = (String, AssayQuantSafety)>,
    ) -> Self {
        let mut codec = Self::new(dim);
        for (slot, safety) in slots {
            codec.record_assay_safety(slot, safety);
        }
        codec
    }

    pub fn record_assay_safety(
        &mut self,
        slot_id: impl Into<String>,
        safety: AssayQuantSafety,
    ) -> bool {
        if !safety.passes() {
            return false;
        }
        self.assay_safe_slots.insert(slot_id.into(), safety);
        true
    }

    pub fn encode_for_slot(&self, slot_id: &str, vec: &[f32]) -> Result<QuantizedVec> {
        self.encode_assay_checked(slot_id, vec, self.assay_safe_slots.contains_key(slot_id))
    }

    pub fn encode_assay_checked(
        &self,
        _slot_id: &str,
        vec: &[f32],
        assay_safe: bool,
    ) -> Result<QuantizedVec> {
        if vec.len() != self.dim {
            return Err(ForgeError::ShapeMismatch {
                expected: vec![self.dim],
                got: vec![vec.len()],
                remediation: "Encode MXFP4 vectors with the codec dimension".to_string(),
            });
        }
        if !assay_safe {
            let blocks = encode_mxfp8(vec)?;
            return Ok(QuantizedVec {
                level: QuantLevel::Bits8Fp,
                dim: self.dim,
                bytes: serialize_mxfp8_blocks(&blocks),
                scale: 0.0,
                seed_id: ZERO_SEED,
            });
        }
        let blocks = encode_mxfp4(vec)?;
        Ok(QuantizedVec {
            level: QuantLevel::Bits4Fp,
            dim: self.dim,
            bytes: serialize_blocks(&blocks),
            scale: 0.0,
            seed_id: ZERO_SEED,
        })
    }
}

impl Quantizer for MxFp4Codec {
    fn encode(&self, vec: &[f32]) -> Result<QuantizedVec> {
        self.encode_for_slot("slot:ph15-placeholder", vec)
    }

    fn decode(&self, qv: &QuantizedVec) -> Result<Vec<f32>> {
        validate_quantized(qv, self.dim, "decode")?;
        match qv.level {
            QuantLevel::Bits4Fp => {
                let blocks = deserialize_blocks(&qv.bytes)?;
                Ok(decode_mxfp4(&blocks, qv.dim))
            }
            QuantLevel::Bits8Fp => {
                let blocks = deserialize_mxfp8_blocks(&qv.bytes)?;
                Ok(decode_mxfp8(&blocks, qv.dim))
            }
            _ => Err(quant_error("decode", "unsupported quant level")),
        }
    }

    fn dot_estimate(&self, a: &QuantizedVec, b: &QuantizedVec) -> Result<f32> {
        validate_quantized(a, self.dim, "dot_estimate")?;
        validate_quantized(b, self.dim, "dot_estimate")?;
        if a.dim != b.dim {
            return Err(ForgeError::ShapeMismatch {
                expected: vec![a.dim],
                got: vec![b.dim],
                remediation: "Compare MXFP4 vectors with the same dimension".to_string(),
            });
        }
        let left = self.decode(a)?;
        let right = self.decode(b)?;
        // Assay admits FP4 slots only after the intelligence-preservation gate,
        // so this path uses the raw decoded fp32 dot without an unbiased fixup.
        Ok(left
            .iter()
            .zip(right.iter())
            .map(|(lhs, rhs)| lhs * rhs)
            .sum())
    }

    fn level(&self) -> QuantLevel {
        QuantLevel::Bits4Fp
    }

    fn dim(&self) -> usize {
        self.dim
    }
}

pub fn serialize_blocks(blocks: &[MxFp4Block]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(blocks.len() * MXFP4_BLOCK_BYTES);
    for block in blocks {
        bytes.extend_from_slice(&block.codes);
        bytes.push(block.scale_e8m0);
    }
    bytes
}

pub fn serialize_mxfp8_blocks(blocks: &[MxFp8Block]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(blocks.len() * MXFP8_BLOCK_BYTES);
    for block in blocks {
        bytes.extend_from_slice(&block.codes);
        bytes.push(block.scale_e8m0);
    }
    bytes
}

pub fn deserialize_blocks(bytes: &[u8]) -> Result<Vec<MxFp4Block>> {
    if !bytes.len().is_multiple_of(MXFP4_BLOCK_BYTES) {
        return Err(quant_error(
            "decode",
            format!(
                "encoded byte length {} is not a multiple of {MXFP4_BLOCK_BYTES}",
                bytes.len()
            ),
        ));
    }
    let mut blocks = Vec::with_capacity(bytes.len() / MXFP4_BLOCK_BYTES);
    for chunk in bytes.chunks_exact(MXFP4_BLOCK_BYTES) {
        let mut codes = [0; MXFP4_PACKED_BYTES];
        codes.copy_from_slice(&chunk[..MXFP4_PACKED_BYTES]);
        blocks.push(MxFp4Block {
            codes,
            scale_e8m0: chunk[MXFP4_PACKED_BYTES],
        });
    }
    Ok(blocks)
}

pub fn deserialize_mxfp8_blocks(bytes: &[u8]) -> Result<Vec<MxFp8Block>> {
    if !bytes.len().is_multiple_of(MXFP8_BLOCK_BYTES) {
        return Err(quant_error(
            "decode",
            format!(
                "encoded byte length {} is not a multiple of {MXFP8_BLOCK_BYTES}",
                bytes.len()
            ),
        ));
    }
    let mut blocks = Vec::with_capacity(bytes.len() / MXFP8_BLOCK_BYTES);
    for chunk in bytes.chunks_exact(MXFP8_BLOCK_BYTES) {
        let mut codes = [0; crate::MXFP8_BLOCK_SIZE];
        codes.copy_from_slice(&chunk[..crate::MXFP8_BLOCK_SIZE]);
        blocks.push(MxFp8Block {
            codes,
            scale_e8m0: chunk[crate::MXFP8_BLOCK_SIZE],
        });
    }
    Ok(blocks)
}

fn validate_quantized(qv: &QuantizedVec, dim: usize, op: &str) -> Result<()> {
    let expected_len = match qv.level {
        QuantLevel::Bits4Fp => qv.dim.div_ceil(crate::MXFP4_BLOCK_SIZE) * MXFP4_BLOCK_BYTES,
        QuantLevel::Bits8Fp => qv.dim.div_ceil(crate::MXFP8_BLOCK_SIZE) * MXFP8_BLOCK_BYTES,
        _ => {
            return Err(quant_error(
                op,
                "MxFp4Codec only supports Bits4Fp and Bits8Fp",
            ));
        }
    };
    if qv.dim != dim {
        return Err(ForgeError::ShapeMismatch {
            expected: vec![dim],
            got: vec![qv.dim],
            remediation: "Decode MXFP4 vectors with the codec dimension".to_string(),
        });
    }
    if qv.bytes.len() != expected_len {
        return Err(quant_error(
            op,
            format!(
                "encoded byte length mismatch: expected {expected_len} got {}",
                qv.bytes.len()
            ),
        ));
    }
    if qv.scale != 0.0 || qv.seed_id != ZERO_SEED {
        return Err(quant_error(
            op,
            "MXFP4 codec expects scale=0.0 and zero seed_id",
        ));
    }
    Ok(())
}

fn quant_error(op: &str, detail: impl Into<String>) -> ForgeError {
    ForgeError::QuantError {
        op: op.to_string(),
        level: "Bits4Fp".to_string(),
        detail: detail.into(),
        remediation: MXFP_REMEDIATION.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn unit_vec(dim: usize) -> Vec<f32> {
        vec![1.0 / (dim as f32).sqrt(); dim]
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let mut dot = 0.0;
        let mut aa = 0.0;
        let mut bb = 0.0;
        for (left, right) in a.iter().zip(b.iter()) {
            dot += left * right;
            aa += left * left;
            bb += right * right;
        }
        dot / (aa.sqrt() * bb.sqrt())
    }

    #[test]
    fn mxfp4_codec_encode_falls_back_to_bits8fp() -> Result<()> {
        let codec = MxFp4Codec::new(128);
        let qv = codec.encode(&unit_vec(128))?;
        assert_eq!(qv.level, QuantLevel::Bits8Fp);
        assert_eq!(qv.scale, 0.0);
        assert_eq!(qv.seed_id, ZERO_SEED);
        println!(
            "mxfp4_codec_encode_fail_closed PASSED level={:?} bytes={} bits={}",
            qv.level,
            qv.bytes.len(),
            qv.level.bits_per_channel()
        );
        Ok(())
    }

    #[test]
    fn mxfp4_codec_explicit_assay_safe_sets_bits4fp() -> Result<()> {
        let codec = MxFp4Codec::new(128);
        let original = unit_vec(128);
        let qv = codec.encode_assay_checked("slot:assay-safe", &original, true)?;
        assert_eq!(qv.level, QuantLevel::Bits4Fp);
        assert_eq!(qv.scale, 0.0);
        assert_eq!(qv.seed_id, ZERO_SEED);
        let decoded = codec.decode(&qv)?;
        let cos = cosine(&original, &decoded);
        assert!(cos >= 0.95, "cosine={cos}");
        println!(
            "mxfp4_codec_assay_safe PASSED cosine={cos:.6} bytes={}",
            qv.bytes.len()
        );
        Ok(())
    }

    #[test]
    fn mxfp4_codec_assay_evidence_controls_slot_admission() -> Result<()> {
        let safe = AssayQuantSafety {
            baseline_bits: 1.0,
            quantized_bits: 0.97,
            cosine: 0.995,
            far_delta: 0.005,
        };
        let unsafe_evidence = AssayQuantSafety {
            baseline_bits: 1.0,
            quantized_bits: 0.70,
            cosine: 0.995,
            far_delta: 0.005,
        };
        let mut codec = MxFp4Codec::new(128);
        assert!(codec.record_assay_safety("slot:safe", safe));
        assert!(!codec.record_assay_safety("slot:unsafe", unsafe_evidence));

        let safe_qv = codec.encode_for_slot("slot:safe", &unit_vec(128))?;
        let unsafe_qv = codec.encode_for_slot("slot:unsafe", &unit_vec(128))?;

        assert_eq!(safe_qv.level, QuantLevel::Bits4Fp);
        assert_eq!(unsafe_qv.level, QuantLevel::Bits8Fp);
        println!(
            "mxfp4_assay_evidence PASSED safe={:?} unsafe={:?}",
            safe_qv.level, unsafe_qv.level
        );
        Ok(())
    }

    #[test]
    fn mxfp4_codec_roundtrip_cosine() -> Result<()> {
        let codec = MxFp4Codec::new(128);
        let original = unit_vec(128);
        let qv = codec.encode_for_slot("slot:unit", &original)?;
        assert_eq!(qv.level, QuantLevel::Bits8Fp);
        let decoded = codec.decode(&qv)?;
        let cos = cosine(&original, &decoded);
        assert!(cos >= 0.99, "cosine={cos}");
        println!(
            "mxfp4_codec_roundtrip PASSED cosine={cos:.6} dim={} bytes={}",
            decoded.len(),
            qv.bytes.len()
        );
        Ok(())
    }

    #[test]
    fn mxfp8_fallback_edges_fail_closed_and_large_dim() -> Result<()> {
        let codec = MxFp4Codec::new(1536);
        let vec = unit_vec(1536);
        let qv = codec.encode(&vec)?;
        assert_eq!(codec.decode(&qv)?.len(), 1536);

        let fallback = codec.encode_assay_checked("slot:unsafe", &vec, false)?;
        assert_eq!(fallback.level, QuantLevel::Bits8Fp);
        let fallback_decoded = codec.decode(&fallback)?;
        let fallback_cosine = cosine(&vec, &fallback_decoded);
        assert!(fallback_cosine >= 0.99, "cosine={fallback_cosine}");

        let mut corrupt = qv.clone();
        corrupt.bytes.push(1);
        let corrupt_err = codec
            .decode(&corrupt)
            .expect_err("corrupt byte length must fail closed");
        println!(
            "mxfp8_fallback PASSED level={:?} bits={} bytes={} cosine={fallback_cosine:.6} corrupt={corrupt_err}",
            fallback.level,
            fallback.level.bits_per_channel(),
            fallback.bytes.len()
        );
        assert!(matches!(corrupt_err, ForgeError::QuantError { .. }));
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        #[test]
        fn mxfp4_codec_roundtrip_preserves_sign(values in proptest::collection::vec(-1.0f32..1.0, 128)) {
            let codec = MxFp4Codec::new(128);
            let qv = codec.encode_assay_checked("slot:proptest-safe", &values, true)?;
            let decoded = codec.decode(&qv)?;
            for (actual, expected) in decoded.iter().zip(values.iter()) {
                if *expected > 0.0 {
                    prop_assert!(*actual > 0.0);
                } else if *expected < 0.0 {
                    prop_assert!(*actual < 0.0);
                } else {
                    prop_assert_eq!(*actual, 0.0);
                }
            }
        }
    }
}
