use crate::quant::{QuantLevel, QuantizedVec, Quantizer, RotationSeed, apply_rotation};
use crate::{ForgeError, Result};

const BITS3P5_CODE_BITS: usize = 7;
const BITS3P5_LEVELS: u16 = 1 << BITS3P5_CODE_BITS;
const BITS2P5_LEVELS: u16 = 5;
const TURBOQUANT_LEVEL_DETAIL: &str = "TurboQuant only supports Bits3p5 and Bits2p5";

#[derive(Clone, Debug)]
pub struct TurboQuantCodec {
    seed: RotationSeed,
    level: QuantLevel,
}

impl TurboQuantCodec {
    pub fn new(seed: RotationSeed, level: QuantLevel) -> Result<Self> {
        validate_level(level)?;
        seed.verify_current_version()?;
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
            return Err(quant_error(
                "new",
                level,
                "rotation seed diagonal must contain only finite +/-1 signs",
            ));
        }
        Ok(Self { seed, level })
    }
}

impl Quantizer for TurboQuantCodec {
    fn encode(&self, vec: &[f32]) -> Result<QuantizedVec> {
        self.seed.verify_current_version()?;
        if vec.len() != self.seed.dim {
            return Err(ForgeError::ShapeMismatch {
                expected: vec![self.seed.dim],
                got: vec![vec.len()],
                remediation: "Encode vectors with the same dim as the rotation seed".to_string(),
            });
        }
        if let Some(idx) = vec.iter().position(|value| !value.is_finite()) {
            return Err(quant_error(
                "encode",
                self.level,
                format!("non-finite input coefficient at index {idx}"),
            ));
        }
        let (bytes, scale) = rotate_and_quantize_scalar(&self.seed, vec, self.level);
        Ok(QuantizedVec {
            level: self.level,
            dim: self.seed.dim,
            bytes,
            scale,
            seed_id: self.seed.id,
        })
    }

    fn decode(&self, qv: &QuantizedVec) -> Result<Vec<f32>> {
        validate_level(qv.level)?;
        if qv.level != self.level {
            return Err(quant_error(
                "decode",
                qv.level,
                format!(
                    "quant level mismatch: expected {:?} got {:?}",
                    self.level, qv.level
                ),
            ));
        }
        if qv.dim != self.seed.dim {
            return Err(ForgeError::ShapeMismatch {
                expected: vec![self.seed.dim],
                got: vec![qv.dim],
                remediation: "Decode with the codec seed used for encode".to_string(),
            });
        }
        if qv.seed_id != self.seed.id {
            return Err(quant_error("decode", qv.level, "rotation seed id mismatch"));
        }
        if !qv.scale.is_finite() || qv.scale < 0.0 {
            return Err(quant_error(
                "decode",
                qv.level,
                "scale must be finite and non-negative",
            ));
        }
        let expected_len = packed_len(qv.dim, qv.level);
        if qv.bytes.len() != expected_len {
            return Err(quant_error(
                "decode",
                qv.level,
                format!(
                    "encoded byte length mismatch: expected {expected_len} got {}",
                    qv.bytes.len()
                ),
            ));
        }
        Ok(dequantize_scalar(&qv.bytes, qv.scale, qv.dim, qv.level))
    }

    fn dot_estimate(&self, a: &QuantizedVec, b: &QuantizedVec) -> Result<f32> {
        let left = self.decode(a)?;
        let right = self.decode(b)?;
        Ok(left
            .iter()
            .zip(right.iter())
            .map(|(a, b)| a * b)
            .sum::<f32>())
    }

    fn level(&self) -> QuantLevel {
        self.level
    }

    fn dim(&self) -> usize {
        self.seed.dim
    }
}

fn rotate_and_quantize_scalar(
    seed: &RotationSeed,
    vec: &[f32],
    level: QuantLevel,
) -> (Vec<u8>, f32) {
    let mut rotated = vec.to_vec();
    apply_rotation(seed, &mut rotated);
    let scale = rotated
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f32, f32::max);
    let codes = quantize_codes(&rotated, scale, level);
    (pack_codes(&codes, level), scale)
}

fn dequantize_scalar(bytes: &[u8], scale: f32, dim: usize, level: QuantLevel) -> Vec<f32> {
    if scale == 0.0 {
        return vec![0.0; dim];
    }
    let codes = unpack_codes(bytes, dim, level);
    let max_code = f32::from(level_steps(level) - 1);
    codes
        .iter()
        .map(|code| f32::from(*code) * (2.0 * scale) / max_code - scale)
        .collect()
}

fn quantize_codes(rotated: &[f32], scale: f32, level: QuantLevel) -> Vec<u16> {
    if scale == 0.0 {
        return vec![0; rotated.len()];
    }
    let max_code = f32::from(level_steps(level) - 1);
    rotated
        .iter()
        .map(|value| {
            (((*value / scale + 1.0) * max_code / 2.0).round()).clamp(0.0, max_code) as u16
        })
        .collect()
}

fn pack_codes(codes: &[u16], level: QuantLevel) -> Vec<u8> {
    match level {
        QuantLevel::Bits3p5 => pack_bits3p5(codes),
        QuantLevel::Bits2p5 => pack_bits2p5(codes),
        _ => unreachable!("TurboQuant level validated before packing"),
    }
}

fn unpack_codes(bytes: &[u8], dim: usize, level: QuantLevel) -> Vec<u16> {
    match level {
        QuantLevel::Bits3p5 => unpack_bits3p5(bytes, dim),
        QuantLevel::Bits2p5 => unpack_bits2p5(bytes, dim),
        _ => unreachable!("TurboQuant level validated before unpacking"),
    }
}

fn pack_bits3p5(codes: &[u16]) -> Vec<u8> {
    let mut out = vec![0; packed_len(codes.len(), QuantLevel::Bits3p5)];
    // Bits3p5 stores one 7-bit scalar code per coordinate. Codes are written
    // little-endian into a bitstream, so 8 values occupy 56 bits = 7 bytes.
    for (idx, code) in codes.iter().enumerate() {
        write_bits(&mut out, idx * BITS3P5_CODE_BITS, BITS3P5_CODE_BITS, *code);
    }
    out
}

fn unpack_bits3p5(bytes: &[u8], dim: usize) -> Vec<u16> {
    (0..dim)
        .map(|idx| read_bits(bytes, idx * BITS3P5_CODE_BITS, BITS3P5_CODE_BITS))
        .collect()
}

fn pack_bits2p5(codes: &[u16]) -> Vec<u8> {
    let mut out = vec![0; packed_len(codes.len(), QuantLevel::Bits2p5)];
    // Bits2p5 stores four base-5 scalar codes in one 10-bit lane:
    // packed = c0 + 5*c1 + 25*c2 + 125*c3. The upper 6 bits of the 2-byte
    // group are padding, giving exactly 4 values per 2 bytes.
    for (group, chunk) in codes.chunks(4).enumerate() {
        let mut packed = 0u16;
        let mut factor = 1u16;
        for code in chunk {
            packed += *code * factor;
            factor *= BITS2P5_LEVELS;
        }
        let base = group * 2;
        out[base] = packed as u8;
        out[base + 1] = (packed >> 8) as u8;
    }
    out
}

fn unpack_bits2p5(bytes: &[u8], dim: usize) -> Vec<u16> {
    let mut codes = Vec::with_capacity(dim);
    for group in 0..dim.div_ceil(4) {
        let base = group * 2;
        let mut packed = u16::from(bytes[base]) | (u16::from(bytes[base + 1]) << 8);
        for _ in 0..4 {
            if codes.len() == dim {
                break;
            }
            codes.push(packed % BITS2P5_LEVELS);
            packed /= BITS2P5_LEVELS;
        }
    }
    codes
}

fn write_bits(out: &mut [u8], offset: usize, width: usize, value: u16) {
    for bit in 0..width {
        if ((value >> bit) & 1) == 1 {
            let absolute = offset + bit;
            out[absolute / 8] |= 1 << (absolute % 8);
        }
    }
}

fn read_bits(bytes: &[u8], offset: usize, width: usize) -> u16 {
    let mut value = 0u16;
    for bit in 0..width {
        let absolute = offset + bit;
        if ((bytes[absolute / 8] >> (absolute % 8)) & 1) == 1 {
            value |= 1 << bit;
        }
    }
    value
}

fn packed_len(dim: usize, level: QuantLevel) -> usize {
    match level {
        QuantLevel::Bits3p5 => (dim * BITS3P5_CODE_BITS).div_ceil(8),
        QuantLevel::Bits2p5 => dim.div_ceil(4) * 2,
        _ => unreachable!("TurboQuant level validated before sizing"),
    }
}

fn level_steps(level: QuantLevel) -> u16 {
    match level {
        QuantLevel::Bits3p5 => BITS3P5_LEVELS,
        QuantLevel::Bits2p5 => BITS2P5_LEVELS,
        _ => unreachable!("TurboQuant level validated before stepping"),
    }
}

fn validate_level(level: QuantLevel) -> Result<()> {
    if matches!(level, QuantLevel::Bits3p5 | QuantLevel::Bits2p5) {
        return Ok(());
    }
    Err(quant_error("new", level, TURBOQUANT_LEVEL_DETAIL))
}

fn quant_error(op: &str, level: QuantLevel, detail: impl Into<String>) -> ForgeError {
    ForgeError::QuantError {
        op: op.to_string(),
        level: format!("{level:?}"),
        detail: detail.into(),
        remediation: "Use finite vectors, matching seeds, and a supported TurboQuant level"
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quant::new_seed;
    use proptest::prelude::*;

    fn rotated(seed: &RotationSeed, vec: &[f32]) -> Vec<f32> {
        let mut out = vec.to_vec();
        apply_rotation(seed, &mut out);
        out
    }

    fn max_abs_delta(left: &[f32], right: &[f32]) -> f32 {
        left.iter()
            .zip(right.iter())
            .map(|(left, right)| (left - right).abs())
            .fold(0.0_f32, f32::max)
    }

    fn bin_width(scale: f32, level: QuantLevel) -> f32 {
        if scale == 0.0 {
            return 0.0;
        }
        2.0 * scale / f32::from(level_steps(level) - 1)
    }

    #[test]
    fn scalar_zero_roundtrip_bits3p5() {
        let seed = new_seed(128, b"tq_zero");
        let codec = TurboQuantCodec::new(seed, QuantLevel::Bits3p5).expect("codec");
        let qv = codec.encode(&vec![0.0; 128]).expect("encode");
        let decoded = codec.decode(&qv).expect("decode");
        let max_err = decoded
            .iter()
            .map(|value| value.abs())
            .fold(0.0_f32, f32::max);
        assert!(max_err <= 1e-2, "{max_err}");
        assert_eq!(qv.scale, 0.0);
        assert_eq!(qv.bytes.len(), 112);
        println!(
            "scalar_zero_roundtrip_bits3p5 PASSED roundtrip max_err={max_err:.6} scale={:.6} len={}",
            qv.scale,
            qv.bytes.len()
        );
    }

    #[test]
    fn scalar_roundtrip_bits3p5() {
        let seed = new_seed(128, b"tq_unit");
        let codec = TurboQuantCodec::new(seed.clone(), QuantLevel::Bits3p5).expect("codec");
        let mut input = vec![0.0; 128];
        input[0] = 1.0;
        let expected = rotated(&seed, &input);
        let qv = codec.encode(&input).expect("encode");
        let decoded = codec.decode(&qv).expect("decode");
        let max_err = max_abs_delta(&decoded, &expected);
        let limit = bin_width(qv.scale, QuantLevel::Bits3p5) * 1.5;
        assert!(max_err <= limit, "max_err={max_err} limit={limit}");
        println!(
            "scalar_roundtrip_bits3p5 PASSED max_err={max_err:.8} bin_width={:.8} scale={:.8} len={}",
            bin_width(qv.scale, QuantLevel::Bits3p5),
            qv.scale,
            qv.bytes.len()
        );
    }

    #[test]
    fn scalar_encode_len_deterministic() {
        let seed = new_seed(128, b"tq_len");
        let vec = vec![0.125; 128];
        let bits3 = TurboQuantCodec::new(seed.clone(), QuantLevel::Bits3p5).expect("bits3");
        let bits2 = TurboQuantCodec::new(seed, QuantLevel::Bits2p5).expect("bits2");
        let first = bits3.encode(&vec).expect("encode first");
        let second = bits3.encode(&vec).expect("encode second");
        let low = bits2.encode(&vec).expect("encode bits2");
        assert_eq!(first.bytes.len(), second.bytes.len());
        assert_eq!(first.bytes.len(), 112);
        assert_eq!(low.bytes.len(), 64);
        println!(
            "scalar_encode_len_deterministic PASSED bytes_len bits3p5={} bits2p5={}",
            first.bytes.len(),
            low.bytes.len()
        );
    }

    #[test]
    fn scalar_edges_dim1_dim1536_and_identical() {
        let one_seed = new_seed(1, b"tq_dim1");
        let one_codec = TurboQuantCodec::new(one_seed.clone(), QuantLevel::Bits3p5).expect("one");
        let one_qv = one_codec.encode(&[2.0]).expect("one encode");
        let one_decoded = one_codec.decode(&one_qv).expect("one decode");
        assert!(max_abs_delta(&one_decoded, &rotated(&one_seed, &[2.0])) <= 1e-6);

        let large_seed = new_seed(1536, b"tq_large");
        let large_codec =
            TurboQuantCodec::new(large_seed, QuantLevel::Bits3p5).expect("large codec");
        let large_qv = large_codec.encode(&vec![0.0; 1536]).expect("large encode");
        let large_decoded = large_codec.decode(&large_qv).expect("large decode");
        assert!(large_decoded.iter().all(|value| value.is_finite()));
        assert_eq!(large_qv.bytes.len(), 1344);

        let same_seed = new_seed(128, b"tq_identical");
        let same_codec =
            TurboQuantCodec::new(same_seed.clone(), QuantLevel::Bits2p5).expect("same");
        let same_vec = vec![0.25; 128];
        let same_qv = same_codec.encode(&same_vec).expect("same encode");
        let same_decoded = same_codec.decode(&same_qv).expect("same decode");
        let same_err = max_abs_delta(&same_decoded, &rotated(&same_seed, &same_vec));
        assert!(same_err <= bin_width(same_qv.scale, QuantLevel::Bits2p5) * 1.5 + 1e-6);
        println!(
            "scalar_edges PASSED dim1_len={} dim1536_len={} identical_bits2p5_len={} max_err={same_err:.8}",
            one_qv.bytes.len(),
            large_qv.bytes.len(),
            same_qv.bytes.len()
        );
    }

    #[test]
    fn scalar_invalid_level_fails_closed() {
        let err = TurboQuantCodec::new(new_seed(8, b"tq_invalid"), QuantLevel::F32)
            .expect_err("F32 unsupported");
        assert!(matches!(err, ForgeError::QuantError { .. }));
        assert!(err.to_string().contains(TURBOQUANT_LEVEL_DETAIL));
        println!("scalar_invalid_level PASSED {err}");
    }

    #[test]
    fn scalar_rejects_non_finite_input() {
        let codec =
            TurboQuantCodec::new(new_seed(8, b"tq_nonfinite"), QuantLevel::Bits3p5).expect("codec");
        let mut vec = vec![0.0; 8];
        vec[3] = f32::NAN;
        let err = codec.encode(&vec).expect_err("NaN must fail closed");
        assert!(matches!(err, ForgeError::QuantError { .. }));
        println!("scalar_non_finite PASSED {err}");
    }

    proptest! {
        #[test]
        fn scalar_bits3p5_random_unit_vectors_stay_within_bound(
            mut values in proptest::collection::vec(-1.0f32..1.0, 128)
        ) {
            let norm = values.iter().map(|value| f64::from(*value) * f64::from(*value)).sum::<f64>().sqrt();
            if norm <= f64::from(f32::EPSILON) {
                values[0] = 1.0;
            } else {
                for value in &mut values {
                    *value /= norm as f32;
                }
            }
            let seed = new_seed(128, b"tq_prop_bound");
            let codec = TurboQuantCodec::new(seed.clone(), QuantLevel::Bits3p5).expect("codec");
            let expected = rotated(&seed, &values);
            let qv = codec.encode(&values).expect("encode");
            let decoded = codec.decode(&qv).expect("decode");
            let max_err = max_abs_delta(&decoded, &expected);
            let limit = qv.scale * 2.0 / (7.0 - 1.0);
            prop_assert!(max_err <= limit + 1e-6, "max_err={max_err} limit={limit}");
        }

        #[test]
        fn scalar_encoded_len_depends_only_on_dim_level(
            dim in 1usize..257,
            use_bits3p5 in any::<bool>()
        ) {
            let level = if use_bits3p5 { QuantLevel::Bits3p5 } else { QuantLevel::Bits2p5 };
            let left = TurboQuantCodec::new(new_seed(dim, b"tq_len_left"), level).expect("left");
            let right = TurboQuantCodec::new(new_seed(dim, b"tq_len_right"), level).expect("right");
            let vec = vec![0.25; dim];
            let left_qv = left.encode(&vec).expect("left encode");
            let right_qv = right.encode(&vec).expect("right encode");
            prop_assert_eq!(left_qv.bytes.len(), right_qv.bytes.len());
            prop_assert_eq!(left_qv.bytes.len(), packed_len(dim, level));
        }
    }
}
