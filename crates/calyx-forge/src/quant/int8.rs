use crate::quant::{QuantLevel, QuantizedVec, Quantizer, SeedId};
use crate::{ForgeError, Result};

const ZERO_SEED: SeedId = [0; 32];
const INT8_REMEDIATION: &str =
    "Use finite dense vectors, matching dimensions, Bits8 payload bytes, and zero seed_id";

#[derive(Clone, Debug)]
pub struct ScalarInt8Codec {
    dim: usize,
}

impl ScalarInt8Codec {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl Quantizer for ScalarInt8Codec {
    fn encode(&self, vec: &[f32]) -> Result<QuantizedVec> {
        if vec.len() != self.dim {
            return Err(ForgeError::ShapeMismatch {
                expected: vec![self.dim],
                got: vec![vec.len()],
                remediation: "Encode scalar INT8 vectors with the codec dimension".to_string(),
            });
        }
        if let Some(idx) = vec.iter().position(|value| !value.is_finite()) {
            return Err(quant_error(
                "encode",
                format!("non-finite input coefficient at index {idx}"),
            ));
        }
        let max_abs = vec.iter().map(|value| value.abs()).fold(0.0_f32, f32::max);
        let scale = if max_abs == 0.0 { 0.0 } else { max_abs / 127.0 };
        let bytes = if scale == 0.0 {
            vec![0; self.dim]
        } else {
            vec.iter()
                .map(|value| ((*value / scale).round_ties_even()).clamp(-127.0, 127.0) as i8 as u8)
                .collect()
        };
        Ok(QuantizedVec {
            level: QuantLevel::Bits8,
            dim: self.dim,
            bytes,
            scale,
            seed_id: ZERO_SEED,
        })
    }

    fn decode(&self, qv: &QuantizedVec) -> Result<Vec<f32>> {
        validate_quantized(qv, self.dim, "decode")?;
        if qv.scale == 0.0 {
            return Ok(vec![0.0; self.dim]);
        }
        Ok(qv
            .bytes
            .iter()
            .map(|byte| (*byte as i8 as f32) * qv.scale)
            .collect())
    }

    fn dot_estimate(&self, a: &QuantizedVec, b: &QuantizedVec) -> Result<f32> {
        validate_quantized(a, self.dim, "dot_estimate")?;
        validate_quantized(b, self.dim, "dot_estimate")?;
        let left = self.decode(a)?;
        let right = self.decode(b)?;
        Ok(left
            .iter()
            .zip(right.iter())
            .map(|(lhs, rhs)| lhs * rhs)
            .sum())
    }

    fn level(&self) -> QuantLevel {
        QuantLevel::Bits8
    }

    fn dim(&self) -> usize {
        self.dim
    }
}

fn validate_quantized(qv: &QuantizedVec, dim: usize, op: &str) -> Result<()> {
    if qv.level != QuantLevel::Bits8 {
        return Err(quant_error(
            op,
            format!("ScalarInt8Codec only supports Bits8, got {:?}", qv.level),
        ));
    }
    if qv.dim != dim {
        return Err(ForgeError::ShapeMismatch {
            expected: vec![dim],
            got: vec![qv.dim],
            remediation: "Decode scalar INT8 vectors with the codec dimension".to_string(),
        });
    }
    if qv.bytes.len() != qv.dim {
        return Err(quant_error(
            op,
            format!(
                "encoded byte length mismatch: expected {} got {}",
                qv.dim,
                qv.bytes.len()
            ),
        ));
    }
    if !qv.scale.is_finite() || qv.scale < 0.0 {
        return Err(quant_error(op, "scale must be finite and non-negative"));
    }
    if qv.seed_id != ZERO_SEED {
        return Err(quant_error(op, "scalar INT8 codec expects zero seed_id"));
    }
    if qv.scale == 0.0 && qv.bytes.iter().any(|byte| *byte != 0) {
        return Err(quant_error(
            op,
            "zero scale requires every encoded INT8 code to be zero",
        ));
    }
    Ok(())
}

fn quant_error(op: &str, detail: impl Into<String>) -> ForgeError {
    ForgeError::QuantError {
        op: op.to_string(),
        level: "Bits8".to_string(),
        detail: detail.into(),
        remediation: INT8_REMEDIATION.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn scalar_int8_roundtrip_preserves_fixture_shape_and_cosine() -> Result<()> {
        let codec = ScalarInt8Codec::new(8);
        let original = vec![0.0, 0.125, -0.25, 0.5, -1.0, 0.75, -0.375, 0.0625];
        let qv = codec.encode(&original)?;
        let decoded = codec.decode(&qv)?;
        assert_eq!(qv.level, QuantLevel::Bits8);
        assert_eq!(qv.bytes.len(), original.len());
        assert_eq!(qv.seed_id, ZERO_SEED);
        assert!(qv.scale > 0.0);
        assert!(cosine(&original, &decoded) > 0.999);
        println!(
            "SCALAR_INT8_ROUNDTRIP PASSED dim={} bytes={} scale={:.9} cosine={:.9} decoded={decoded:?}",
            qv.dim,
            qv.bytes.len(),
            qv.scale,
            cosine(&original, &decoded),
        );
        Ok(())
    }

    #[test]
    fn scalar_int8_rejects_corrupt_zero_scale_payload() -> Result<()> {
        let codec = ScalarInt8Codec::new(4);
        let mut qv = codec.encode(&[0.0; 4])?;
        assert_eq!(qv.scale, 0.0);
        qv.bytes[1] = 7;
        let err = codec
            .decode(&qv)
            .expect_err("zero-scale nonzero code must fail closed");
        assert!(matches!(err, ForgeError::QuantError { .. }));
        println!("SCALAR_INT8_CORRUPT_ZERO_SCALE PASSED err={err}");
        Ok(())
    }
}
