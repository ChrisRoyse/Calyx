use crate::quant::qjl::{
    append_qjl_section, dot_estimate_unbiased, encode_qjl_residual, read_qjl_section,
};
use crate::quant::{
    QuantLevel, QuantizedVec, Quantizer, RotationSeed, apply_inverse_rotation, apply_rotation,
    new_seed,
};
use crate::{ForgeError, Result};

const BITS3P5_CODE_BITS: usize = 7;
const BITS3P5_LEVELS: u16 = 1 << BITS3P5_CODE_BITS;
const BITS2P5_LEVELS: u16 = 5;
const TURBOQUANT_LEVEL_DETAIL: &str = "TurboQuant only supports Bits3p5 and Bits2p5";

#[derive(Clone, Debug)]
pub struct TurboQuantCodec {
    seed: RotationSeed,
    rademacher: RotationSeed,
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
        let rademacher = derive_rademacher_seed(&seed);
        Ok(Self {
            seed,
            rademacher,
            level,
        })
    }

    pub(crate) fn rademacher(&self) -> &RotationSeed {
        &self.rademacher
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
            return Err(ForgeError::NumericalInvariant {
                op: "turboquant_encode".to_string(),
                detail: format!("non-finite input coefficient at index {idx}"),
                remediation: "Reject NaN/Inf vectors before quantization".to_string(),
            });
        }
        let scalar = rotate_quantize_scalar_parts(&self.seed, vec, self.level);
        let residual = encode_qjl_residual(&scalar.rotated, &scalar.decoded, &self.rademacher);
        let mut bytes = scalar.bytes;
        append_qjl_section(&mut bytes, &residual);
        Ok(QuantizedVec {
            level: self.level,
            dim: self.seed.dim,
            bytes,
            scale: scalar.scale,
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
            return Err(quant_error("decode", qv.level, "seed_id mismatch"));
        }
        if !qv.scale.is_finite() || qv.scale < 0.0 {
            return Err(quant_error(
                "decode",
                qv.level,
                "scale must be finite and non-negative",
            ));
        }
        let expected_len = packed_len(qv.dim, qv.level);
        if qv.bytes.len() < expected_len {
            return Err(quant_error(
                "decode",
                qv.level,
                format!(
                    "encoded byte length mismatch: expected at least {expected_len} got {}",
                    qv.bytes.len()
                ),
            ));
        }
        if qv.bytes.len() > expected_len {
            read_qjl_section(&qv.bytes, expected_len, qv.dim)?;
        }
        let mut decoded = dequantize_scalar(&qv.bytes[..expected_len], qv.scale, qv.dim, qv.level);
        apply_inverse_rotation(&self.seed, &mut decoded);
        Ok(decoded)
    }

    fn dot_estimate(&self, a: &QuantizedVec, b: &QuantizedVec) -> Result<f32> {
        dot_estimate_unbiased(self, a, b)
    }

    fn level(&self) -> QuantLevel {
        self.level
    }

    fn dim(&self) -> usize {
        self.seed.dim
    }
}

struct ScalarQuantized {
    bytes: Vec<u8>,
    scale: f32,
    rotated: Vec<f32>,
    decoded: Vec<f32>,
}

#[allow(dead_code)]
fn rotate_and_quantize_scalar(
    seed: &RotationSeed,
    vec: &[f32],
    level: QuantLevel,
) -> (Vec<u8>, f32) {
    let scalar = rotate_quantize_scalar_parts(seed, vec, level);
    (scalar.bytes, scalar.scale)
}

fn rotate_quantize_scalar_parts(
    seed: &RotationSeed,
    vec: &[f32],
    level: QuantLevel,
) -> ScalarQuantized {
    let mut rotated = vec.to_vec();
    apply_rotation(seed, &mut rotated);
    let scale = rotated
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f32, f32::max);
    let codes = quantize_codes(&rotated, scale, level);
    let bytes = pack_codes(&codes, level);
    let decoded = dequantize_scalar(&bytes, scale, vec.len(), level);
    ScalarQuantized {
        bytes,
        scale,
        rotated,
        decoded,
    }
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

pub(crate) fn packed_len(dim: usize, level: QuantLevel) -> usize {
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

fn derive_rademacher_seed(seed: &RotationSeed) -> RotationSeed {
    let mut entropy = Vec::with_capacity(42);
    entropy.extend_from_slice(b"calyx-qjl-rademacher-v1");
    entropy.extend_from_slice(&seed.id);
    entropy.push(seed.version);
    new_seed(seed.dim, &entropy)
}

#[cfg(test)]
mod tests;
