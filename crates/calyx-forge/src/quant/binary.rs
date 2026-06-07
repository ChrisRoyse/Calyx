use crate::cpu::check_finite;
use crate::quant::{
    QuantLevel, QuantizedVec, Quantizer, RotationSeed, apply_inverse_rotation, apply_rotation,
};
use crate::{ForgeError, Result};

const BINARY_LEVEL_DETAIL: &str = "BinaryCodec only supports Bits1";
const BINARY_REMEDIATION: &str =
    "Use finite vectors, matching seeds, and Bits1 binary quantized vectors";

#[derive(Clone, Debug)]
pub struct BinaryCodec {
    seed: RotationSeed,
}

impl BinaryCodec {
    pub fn new(seed: RotationSeed) -> Result<Self> {
        validate_seed(&seed)?;
        Ok(Self { seed })
    }

    pub fn seed(&self) -> &RotationSeed {
        &self.seed
    }
}

impl Quantizer for BinaryCodec {
    fn encode(&self, vec: &[f32]) -> Result<QuantizedVec> {
        self.seed.verify_current_version()?;
        if vec.len() != self.seed.dim {
            return Err(ForgeError::ShapeMismatch {
                expected: vec![self.seed.dim],
                got: vec![vec.len()],
                remediation: "Encode vectors with the same dim as the rotation seed".to_string(),
            });
        }
        check_finite(vec, "binary_encode")?;
        let mut rotated = vec.to_vec();
        apply_rotation(&self.seed, &mut rotated);
        Ok(QuantizedVec {
            level: QuantLevel::Bits1,
            dim: self.seed.dim,
            bytes: pack_sign_bits(&rotated),
            scale: binary_amplitude(self.seed.dim),
            seed_id: self.seed.id,
        })
    }

    fn decode(&self, qv: &QuantizedVec) -> Result<Vec<f32>> {
        validate_quantized(qv, "decode")?;
        if qv.dim != self.seed.dim {
            return Err(ForgeError::ShapeMismatch {
                expected: vec![self.seed.dim],
                got: vec![qv.dim],
                remediation: "Decode with the binary codec seed used for encode".to_string(),
            });
        }
        if qv.seed_id != self.seed.id {
            return Err(binary_error("decode", qv.level, "seed_id mismatch"));
        }
        let amplitude = binary_amplitude(qv.dim);
        let mut approx = (0..qv.dim)
            .map(|idx| {
                if read_bit(&qv.bytes, idx) {
                    amplitude
                } else {
                    -amplitude
                }
            })
            .collect::<Vec<_>>();
        apply_inverse_rotation(&self.seed, &mut approx);
        Ok(approx)
    }

    fn dot_estimate(&self, a: &QuantizedVec, b: &QuantizedVec) -> Result<f32> {
        hamming_dot_estimate(a, b)
    }

    fn level(&self) -> QuantLevel {
        QuantLevel::Bits1
    }

    fn dim(&self) -> usize {
        self.seed.dim
    }
}

pub fn hamming_dot_estimate(a: &QuantizedVec, b: &QuantizedVec) -> Result<f32> {
    validate_quantized(a, "hamming_dot_estimate")?;
    validate_quantized(b, "hamming_dot_estimate")?;
    if a.dim != b.dim {
        return Err(ForgeError::ShapeMismatch {
            expected: vec![a.dim],
            got: vec![b.dim],
            remediation: "Compare binary vectors with the same dimension".to_string(),
        });
    }
    if a.seed_id != b.seed_id {
        return Err(binary_error(
            "hamming_dot_estimate",
            a.level,
            "seed_id mismatch in hamming_dot_estimate",
        ));
    }
    let mismatches = (0..a.dim)
        .filter(|idx| read_bit(&a.bytes, *idx) != read_bit(&b.bytes, *idx))
        .count();
    Ok(1.0 - 2.0 * mismatches as f32 / a.dim as f32)
}

pub fn binary_prefilter(
    query: &QuantizedVec,
    candidates: &[QuantizedVec],
    keep: usize,
) -> Result<Vec<usize>> {
    validate_quantized(query, "binary_prefilter")?;
    if keep == 0 || candidates.is_empty() {
        return Ok(Vec::new());
    }
    let mut scored = candidates
        .iter()
        .enumerate()
        .map(|(idx, candidate)| hamming_dot_estimate(query, candidate).map(|score| (idx, score)))
        .collect::<Result<Vec<_>>>()?;
    scored.sort_by(|(left_idx, left_score), (right_idx, right_score)| {
        right_score
            .total_cmp(left_score)
            .then_with(|| left_idx.cmp(right_idx))
    });
    Ok(scored
        .into_iter()
        .take(keep.min(candidates.len()))
        .map(|(idx, _)| idx)
        .collect())
}

fn validate_seed(seed: &RotationSeed) -> Result<()> {
    seed.verify_current_version()?;
    if seed.dim == 0 {
        return Err(binary_error(
            "new",
            QuantLevel::Bits1,
            "dim must be non-zero",
        ));
    }
    if seed.diagonal.len() != seed.dim {
        return Err(ForgeError::ShapeMismatch {
            expected: vec![seed.dim],
            got: vec![seed.diagonal.len()],
            remediation: "Load a rotation seed whose diagonal length matches dim".to_string(),
        });
    }
    if seed
        .diagonal
        .iter()
        .any(|sign| !sign.is_finite() || (*sign != 1.0 && *sign != -1.0))
    {
        return Err(binary_error(
            "new",
            QuantLevel::Bits1,
            "rotation seed diagonal must contain only finite +/-1 signs",
        ));
    }
    Ok(())
}

fn validate_quantized(qv: &QuantizedVec, op: &str) -> Result<()> {
    if qv.level != QuantLevel::Bits1 {
        return Err(binary_error(op, qv.level, BINARY_LEVEL_DETAIL));
    }
    if qv.dim == 0 {
        return Err(binary_error(op, qv.level, "dim must be non-zero"));
    }
    let expected_len = packed_len(qv.dim);
    if qv.bytes.len() != expected_len {
        return Err(binary_error(
            op,
            qv.level,
            format!(
                "encoded byte length mismatch: expected {expected_len} got {}",
                qv.bytes.len()
            ),
        ));
    }
    if !qv.scale.is_finite() || qv.scale < 0.0 {
        return Err(binary_error(
            op,
            qv.level,
            "scale must be finite and non-negative",
        ));
    }
    if has_nonzero_padding(&qv.bytes, qv.dim) {
        return Err(binary_error(op, qv.level, "non-zero padding bits"));
    }
    Ok(())
}

fn pack_sign_bits(rotated: &[f32]) -> Vec<u8> {
    let mut bytes = vec![0; packed_len(rotated.len())];
    for (idx, value) in rotated.iter().enumerate() {
        if *value > 0.0 {
            bytes[idx / 8] |= 1 << (idx % 8);
        }
    }
    bytes
}

fn read_bit(bytes: &[u8], idx: usize) -> bool {
    ((bytes[idx / 8] >> (idx % 8)) & 1) == 1
}

fn packed_len(dim: usize) -> usize {
    dim.div_ceil(8)
}

fn binary_amplitude(dim: usize) -> f32 {
    1.0 / (dim as f32).sqrt()
}

fn has_nonzero_padding(bytes: &[u8], dim: usize) -> bool {
    let padding_bits = bytes.len() * 8 - dim;
    if padding_bits == 0 {
        return false;
    }
    let used_bits = 8 - padding_bits;
    let padding_mask = !((1u16 << used_bits) - 1) as u8;
    bytes.last().is_some_and(|last| (*last & padding_mask) != 0)
}

fn binary_error(op: &str, level: QuantLevel, detail: impl Into<String>) -> ForgeError {
    ForgeError::QuantError {
        op: op.to_string(),
        level: format!("{level:?}"),
        detail: detail.into(),
        remediation: BINARY_REMEDIATION.to_string(),
    }
}

#[cfg(test)]
mod tests;
