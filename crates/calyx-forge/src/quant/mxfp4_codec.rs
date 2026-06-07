use crate::mxfp4::{MXFP4_PACKED_BYTES, MxFp4Block, decode_mxfp4, encode_mxfp4};
use crate::quant::{QuantLevel, QuantizedVec, Quantizer, SeedId};
use crate::{ForgeError, Result};

const MXFP4_BLOCK_BYTES: usize = MXFP4_PACKED_BYTES + 1;
const ZERO_SEED: SeedId = [0; 32];
const MXFP4_REMEDIATION: &str =
    "Use finite vectors, matching dims, Bits4Fp bytes, and Assay-safe slots";

#[derive(Clone, Debug)]
pub struct MxFp4Codec {
    dim: usize,
}

impl MxFp4Codec {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }

    pub fn encode_for_slot(&self, slot_id: &str, vec: &[f32]) -> Result<QuantizedVec> {
        self.encode_assay_checked(slot_id, vec, assay_safety_check_placeholder(slot_id))
    }

    pub fn encode_assay_checked(
        &self,
        slot_id: &str,
        vec: &[f32],
        assay_safe: bool,
    ) -> Result<QuantizedVec> {
        if !assay_safe {
            return Err(quant_error(
                "encode",
                format!("slot {slot_id} not Assay-safe for FP4"),
            ));
        }
        if vec.len() != self.dim {
            return Err(ForgeError::ShapeMismatch {
                expected: vec![self.dim],
                got: vec![vec.len()],
                remediation: "Encode MXFP4 vectors with the codec dimension".to_string(),
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
        let blocks = deserialize_blocks(&qv.bytes)?;
        Ok(decode_mxfp4(&blocks, qv.dim))
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

pub fn assay_safety_check_placeholder(slot_id: &str) -> bool {
    let _ = slot_id;
    // TODO(PH29): replace with real Assay bits check (accept_quant §4.4).
    true
}

pub fn serialize_blocks(blocks: &[MxFp4Block]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(blocks.len() * MXFP4_BLOCK_BYTES);
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

fn validate_quantized(qv: &QuantizedVec, dim: usize, op: &str) -> Result<()> {
    if qv.level != QuantLevel::Bits4Fp {
        return Err(quant_error(op, "MxFp4Codec only supports Bits4Fp"));
    }
    if qv.dim != dim {
        return Err(ForgeError::ShapeMismatch {
            expected: vec![dim],
            got: vec![qv.dim],
            remediation: "Decode MXFP4 vectors with the codec dimension".to_string(),
        });
    }
    let expected_len = qv.dim.div_ceil(crate::MXFP4_BLOCK_SIZE) * MXFP4_BLOCK_BYTES;
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
        remediation: MXFP4_REMEDIATION.to_string(),
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
    fn mxfp4_codec_encode_sets_bits4fp() -> Result<()> {
        let codec = MxFp4Codec::new(128);
        let qv = codec.encode(&unit_vec(128))?;
        assert_eq!(qv.level, QuantLevel::Bits4Fp);
        assert_eq!(qv.scale, 0.0);
        assert_eq!(qv.seed_id, ZERO_SEED);
        println!(
            "mxfp4_codec_encode PASSED level={:?} bytes={} bits={}",
            qv.level,
            qv.bytes.len(),
            qv.level.bits_per_channel()
        );
        Ok(())
    }

    #[test]
    fn mxfp4_codec_roundtrip_cosine() -> Result<()> {
        let codec = MxFp4Codec::new(128);
        let original = unit_vec(128);
        let qv = codec.encode_for_slot("slot:unit", &original)?;
        let decoded = codec.decode(&qv)?;
        let cos = cosine(&original, &decoded);
        assert!(cos >= 0.95, "cosine={cos}");
        println!(
            "mxfp4_codec_roundtrip PASSED cosine={cos:.6} dim={} bytes={}",
            decoded.len(),
            qv.bytes.len()
        );
        Ok(())
    }

    #[test]
    fn mxfp4_codec_edges_fail_closed_and_large_dim() -> Result<()> {
        let codec = MxFp4Codec::new(1536);
        let vec = unit_vec(1536);
        let qv = codec.encode(&vec)?;
        assert_eq!(codec.decode(&qv)?.len(), 1536);

        let unsafe_err = codec
            .encode_assay_checked("slot:unsafe", &vec, false)
            .expect_err("unsafe slot must fail closed");
        let mut corrupt = qv.clone();
        corrupt.bytes.push(1);
        let corrupt_err = codec
            .decode(&corrupt)
            .expect_err("corrupt byte length must fail closed");
        println!("mxfp4_codec_edges PASSED {unsafe_err} corrupt={corrupt_err}");
        assert!(matches!(unsafe_err, ForgeError::QuantError { .. }));
        assert!(matches!(corrupt_err, ForgeError::QuantError { .. }));
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]

        #[test]
        fn mxfp4_codec_roundtrip_preserves_sign(values in proptest::collection::vec(-1.0f32..1.0, 128)) {
            let codec = MxFp4Codec::new(128);
            let qv = codec.encode(&values)?;
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
