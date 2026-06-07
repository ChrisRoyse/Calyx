use crate::Result;
use crate::cpu::check_finite;

pub const MXFP8_BLOCK_SIZE: usize = 32;
pub const MXFP8_BLOCK_BYTES: usize = MXFP8_BLOCK_SIZE + 1;

const E8M0_EXP_BIAS: i32 = 127;
const E4M3_EXP_BIAS: i32 = 7;
const E4M3_EXP_MASK: u8 = 0x78;
const E4M3_MANT_MASK: u8 = 0x07;
const E4M3_SIGN_MASK: u8 = 0x80;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MxFp8Block {
    pub codes: [u8; MXFP8_BLOCK_SIZE],
    pub scale_e8m0: u8,
}

pub fn encode_mxfp8_block(block: &[f32; MXFP8_BLOCK_SIZE]) -> Result<MxFp8Block> {
    check_finite(block, "mxfp8_encode_block")?;
    let abs_max = block
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f32, f32::max);
    let scale_e8m0 = scale_byte(abs_max);
    let scale = e8m0_scale(scale_e8m0);
    let mut codes = [0; MXFP8_BLOCK_SIZE];
    for (slot, value) in codes.iter_mut().zip(block.iter()) {
        *slot = encode_e4m3(*value / scale);
    }
    Ok(MxFp8Block { codes, scale_e8m0 })
}

pub fn decode_mxfp8_block(block: &MxFp8Block) -> [f32; MXFP8_BLOCK_SIZE] {
    let scale = e8m0_scale(block.scale_e8m0);
    let mut decoded = [0.0; MXFP8_BLOCK_SIZE];
    for (slot, code) in decoded.iter_mut().zip(block.codes.iter()) {
        *slot = decode_e4m3(*code) * scale;
    }
    decoded
}

pub fn encode_mxfp8(vec: &[f32]) -> Result<Vec<MxFp8Block>> {
    check_finite(vec, "mxfp8_encode")?;
    if vec.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(vec.len().div_ceil(MXFP8_BLOCK_SIZE));
    for chunk in vec.chunks(MXFP8_BLOCK_SIZE) {
        let mut block = [0.0; MXFP8_BLOCK_SIZE];
        block[..chunk.len()].copy_from_slice(chunk);
        out.push(encode_mxfp8_block(&block)?);
    }
    Ok(out)
}

pub fn decode_mxfp8(blocks: &[MxFp8Block], original_dim: usize) -> Vec<f32> {
    let mut decoded = Vec::with_capacity(blocks.len() * MXFP8_BLOCK_SIZE);
    for block in blocks {
        decoded.extend_from_slice(&decode_mxfp8_block(block));
    }
    decoded.truncate(original_dim);
    decoded
}

pub fn e8m0_scale(scale_e8m0: u8) -> f32 {
    2.0_f32.powi(i32::from(scale_e8m0) - E8M0_EXP_BIAS)
}

fn scale_byte(abs_max: f32) -> u8 {
    if abs_max == 0.0 {
        return 0;
    }
    let exponent = (abs_max.log2().floor() as i32).clamp(-127, 127);
    (exponent + E8M0_EXP_BIAS) as u8
}

// E4M3 byte layout: bit 7 sign, bits 6..3 exponent with bias 7,
// bits 2..0 mantissa. This implementation treats exponent 15 as finite,
// fail-closed by never producing NaN/Inf codes.
fn encode_e4m3(value: f32) -> u8 {
    if value == 0.0 {
        return 0;
    }
    let mut best_code = 0;
    let mut best_err = f32::INFINITY;
    for code in 0u8..=u8::MAX {
        let decoded = decode_e4m3(code);
        let err = (decoded - value).abs();
        if err < best_err {
            best_err = err;
            best_code = code;
        }
    }
    best_code
}

fn decode_e4m3(code: u8) -> f32 {
    let sign = if code & E4M3_SIGN_MASK == 0 {
        1.0
    } else {
        -1.0
    };
    let exp = (code & E4M3_EXP_MASK) >> 3;
    let mant = code & E4M3_MANT_MASK;
    if exp == 0 {
        if mant == 0 {
            return sign * 0.0;
        }
        return sign * (f32::from(mant) / 8.0) * 2.0_f32.powi(1 - E4M3_EXP_BIAS);
    }
    let significand = 1.0 + f32::from(mant) / 8.0;
    sign * significand * 2.0_f32.powi(i32::from(exp) - E4M3_EXP_BIAS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ForgeError;

    fn filled(value: f32) -> [f32; MXFP8_BLOCK_SIZE] {
        [value; MXFP8_BLOCK_SIZE]
    }

    fn unit_vec(dim: usize) -> Vec<f32> {
        vec![1.0 / (dim as f32).sqrt(); dim]
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a
            .iter()
            .zip(b.iter())
            .map(|(left, right)| left * right)
            .sum();
        let aa: f32 = a.iter().map(|value| value * value).sum();
        let bb: f32 = b.iter().map(|value| value * value).sum();
        dot / (aa.sqrt() * bb.sqrt())
    }

    #[test]
    fn mxfp8_encode_ones_block() -> Result<()> {
        let encoded = encode_mxfp8_block(&filled(1.0))?;
        let decoded = decode_mxfp8_block(&encoded);
        assert_eq!(encoded.scale_e8m0, 127);
        assert!(encoded.codes.iter().all(|code| *code == 0x38));
        for value in decoded {
            assert!((value - 1.0).abs() <= f32::EPSILON);
        }
        println!(
            "mxfp8_encode_ones PASSED scale_e8m0={} code=0x{:02x}",
            encoded.scale_e8m0, encoded.codes[0]
        );
        Ok(())
    }

    #[test]
    fn mxfp8_codec_roundtrip() -> Result<()> {
        let original = unit_vec(128);
        let blocks = encode_mxfp8(&original)?;
        let decoded = decode_mxfp8(&blocks, original.len());
        let cos = cosine(&original, &decoded);
        assert!(cos >= 0.99, "cosine={cos}");
        println!(
            "mxfp8_codec_roundtrip PASSED cosine={cos:.6} blocks={} scale_e8m0={}",
            blocks.len(),
            blocks[0].scale_e8m0
        );
        Ok(())
    }

    #[test]
    fn mxfp8_edges_dim1536_outlier_and_nan() -> Result<()> {
        let dim1536 = unit_vec(1536);
        let blocks = encode_mxfp8(&dim1536)?;
        assert_eq!(decode_mxfp8(&blocks, 1536).len(), 1536);

        let mut outlier = filled(0.125);
        outlier[11] = 64.0;
        let encoded = encode_mxfp8_block(&outlier)?;
        let decoded = decode_mxfp8_block(&encoded);
        assert_eq!(encoded.scale_e8m0, 133);
        assert!((decoded[11] - 64.0).abs() <= f32::EPSILON);

        let mut bad = filled(1.0);
        bad[7] = f32::NAN;
        let err = encode_mxfp8_block(&bad).expect_err("NaN must fail closed");
        assert!(matches!(err, ForgeError::NumericalInvariant { .. }));
        println!(
            "mxfp8_edges PASSED dim1536_blocks={} outlier_scale_e8m0={} {err}",
            blocks.len(),
            encoded.scale_e8m0
        );
        Ok(())
    }
}
