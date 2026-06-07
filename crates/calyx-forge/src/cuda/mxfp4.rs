use crate::Result;
use crate::cpu::check_finite;

pub const MXFP4_BLOCK_SIZE: usize = 32;
pub const MXFP4_PACKED_BYTES: usize = MXFP4_BLOCK_SIZE / 2;
const MXFP4_EXP_BIAS: i32 = 127;
const MXFP4_MAX_SIGNED_CODE: i8 = 7;
const MXFP4_ZERO_CODE: u8 = 7;
const MXFP4_NAN_CODE: u8 = 15;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MxFp4Block {
    pub codes: [u8; MXFP4_PACKED_BYTES],
    pub scale_e8m0: u8,
}

pub fn encode_mxfp4_block(block: &[f32; MXFP4_BLOCK_SIZE]) -> Result<MxFp4Block> {
    check_finite(block, "mxfp4_encode_block")?;
    let abs_max = block
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f32, f32::max);
    let scale_e8m0 = scale_byte(abs_max);
    let scale = e8m0_scale(scale_e8m0);
    let mut codes = [0; MXFP4_PACKED_BYTES];
    for (idx, value) in block.iter().enumerate() {
        let code = quantize_code(*value, scale);
        if idx.is_multiple_of(2) {
            codes[idx / 2] |= code;
        } else {
            codes[idx / 2] |= code << 4;
        }
    }
    Ok(MxFp4Block { codes, scale_e8m0 })
}

pub fn decode_mxfp4_block(block: &MxFp4Block) -> [f32; MXFP4_BLOCK_SIZE] {
    let mut decoded = [0.0; MXFP4_BLOCK_SIZE];
    let scale = e8m0_scale(block.scale_e8m0);
    for (idx, slot) in decoded.iter_mut().enumerate() {
        *slot = decode_code(nibble_at(&block.codes, idx), scale);
    }
    decoded
}

pub fn encode_mxfp4(vec: &[f32]) -> Result<Vec<MxFp4Block>> {
    check_finite(vec, "mxfp4_encode")?;
    if vec.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(vec.len().div_ceil(MXFP4_BLOCK_SIZE));
    for chunk in vec.chunks(MXFP4_BLOCK_SIZE) {
        let mut block = [0.0; MXFP4_BLOCK_SIZE];
        block[..chunk.len()].copy_from_slice(chunk);
        out.push(encode_mxfp4_block(&block)?);
    }
    Ok(out)
}

pub fn decode_mxfp4(blocks: &[MxFp4Block], original_dim: usize) -> Vec<f32> {
    let mut decoded = Vec::with_capacity(blocks.len() * MXFP4_BLOCK_SIZE);
    for block in blocks {
        decoded.extend_from_slice(&decode_mxfp4_block(block));
    }
    decoded.truncate(original_dim);
    decoded
}

pub fn e8m0_scale(scale_e8m0: u8) -> f32 {
    2.0_f32.powi(i32::from(scale_e8m0) - MXFP4_EXP_BIAS)
}

fn scale_byte(abs_max: f32) -> u8 {
    if abs_max == 0.0 {
        return 0;
    }
    let exponent = (abs_max.log2().floor() as i32).clamp(-127, 127);
    (exponent + MXFP4_EXP_BIAS) as u8
}

fn quantize_code(value: f32, scale: f32) -> u8 {
    if value == 0.0 {
        return MXFP4_ZERO_CODE;
    }
    let mut signed =
        ((value / scale).clamp(-1.0, 1.0) * f32::from(MXFP4_MAX_SIGNED_CODE)).round() as i8;
    if signed == 0 {
        signed = if value.is_sign_positive() { 1 } else { -1 };
    }
    (signed + MXFP4_ZERO_CODE as i8) as u8
}

fn decode_code(code: u8, scale: f32) -> f32 {
    if code == MXFP4_NAN_CODE {
        return 0.0;
    }
    let signed = code.clamp(0, 14) as i8 - MXFP4_ZERO_CODE as i8;
    f32::from(signed) * scale / f32::from(MXFP4_MAX_SIGNED_CODE)
}

fn nibble_at(codes: &[u8; MXFP4_PACKED_BYTES], idx: usize) -> u8 {
    let byte = codes[idx / 2];
    if idx.is_multiple_of(2) {
        byte & 0x0f
    } else {
        byte >> 4
    }
}

#[cfg(test)]
fn unpack_codes(block: &MxFp4Block) -> [u8; MXFP4_BLOCK_SIZE] {
    let mut out = [0; MXFP4_BLOCK_SIZE];
    for (idx, slot) in out.iter_mut().enumerate() {
        *slot = nibble_at(&block.codes, idx);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ForgeError;
    use proptest::prelude::*;

    fn filled(value: f32) -> [f32; MXFP4_BLOCK_SIZE] {
        [value; MXFP4_BLOCK_SIZE]
    }

    fn max_abs_error(left: &[f32], right: &[f32]) -> f32 {
        left.iter()
            .zip(right.iter())
            .map(|(a, b)| (*a - *b).abs())
            .fold(0.0, f32::max)
    }

    fn finite_block(values: Vec<f32>) -> [f32; MXFP4_BLOCK_SIZE] {
        let mut block = [0.0; MXFP4_BLOCK_SIZE];
        for (slot, value) in block.iter_mut().zip(values) {
            *slot = value;
        }
        block
    }

    fn assert_no_nan(values: &[f32]) {
        assert!(values.iter().all(|value| !value.is_nan()));
    }

    #[test]
    fn encode_ones_block() {
        let encoded = encode_mxfp4_block(&filled(1.0)).expect("encode ones");
        let codes = unpack_codes(&encoded);
        let decoded = decode_mxfp4_block(&encoded);
        assert_eq!(encoded.scale_e8m0, 127);
        assert!(codes.iter().all(|code| *code == 14));
        assert_eq!(decoded, filled(1.0));
        println!(
            "encode_ones_block PASSED scale_e8m0={} codes={:?}",
            encoded.scale_e8m0, codes
        );
    }

    #[test]
    fn encode_zero_block() {
        let encoded = encode_mxfp4_block(&filled(0.0)).expect("encode zeros");
        let codes = unpack_codes(&encoded);
        let decoded = decode_mxfp4_block(&encoded);
        assert_eq!(encoded.scale_e8m0, 0);
        assert!(codes.iter().all(|code| *code == MXFP4_ZERO_CODE));
        assert_eq!(decoded, filled(0.0));
        println!(
            "encode_zero_block PASSED scale_e8m0={} codes={:?}",
            encoded.scale_e8m0, codes
        );
    }

    #[test]
    fn encode_decode_roundtrip_edges() {
        let mut outlier = [0.125; MXFP4_BLOCK_SIZE];
        outlier[17] = 64.0;
        let encoded = encode_mxfp4_block(&outlier).expect("outlier encode");
        let decoded = decode_mxfp4_block(&encoded);
        assert_eq!(encoded.scale_e8m0, 133);
        assert!((decoded[17] - 64.0).abs() <= f32::EPSILON);

        let padded = encode_mxfp4(&[1.0; 31]).expect("padded encode");
        assert_eq!(padded.len(), 1);
        let padded_decoded = decode_mxfp4(&padded, 31);
        assert_eq!(padded_decoded.len(), 31);

        let equal = encode_mxfp4_block(&filled(-2.0)).expect("equal encode");
        let equal_decoded = decode_mxfp4_block(&equal);
        assert_eq!(equal.scale_e8m0, 128);
        assert!(
            equal_decoded
                .iter()
                .all(|value| (*value + 2.0).abs() <= f32::EPSILON)
        );
        assert_no_nan(&decoded);
        assert_no_nan(&padded_decoded);
        assert_no_nan(&equal_decoded);
        println!(
            "encode_decode_roundtrip PASSED outlier_scale_e8m0={} padded_blocks={} equal_scale_e8m0={} max_err={:.6}",
            encoded.scale_e8m0,
            padded.len(),
            equal.scale_e8m0,
            max_abs_error(&outlier, &decoded)
        );
    }

    #[test]
    fn mxfp4_fail_closed_nan_and_reserved_code() {
        let mut block = filled(1.0);
        block[3] = f32::NAN;
        let err = encode_mxfp4_block(&block).expect_err("NaN must fail closed");
        assert!(matches!(err, ForgeError::NumericalInvariant { .. }));

        let mut encoded = encode_mxfp4_block(&filled(1.0)).expect("encode");
        encoded.codes[0] = 0xff;
        let decoded = decode_mxfp4_block(&encoded);
        assert_eq!(decoded[0], 0.0);
        assert_eq!(decoded[1], 0.0);
        assert_no_nan(&decoded);
        println!(
            "mxfp4_fail_closed PASSED {err} reserved_decode={:?}",
            &decoded[..2]
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn mxfp4_positive_roundtrip_error_is_bounded(values in proptest::collection::vec(0.0f32..64.0, MXFP4_BLOCK_SIZE)) {
            let block = finite_block(values);
            let encoded = encode_mxfp4_block(&block).expect("encode");
            let decoded = decode_mxfp4_block(&encoded);
            let scale = e8m0_scale(encoded.scale_e8m0);
            for (actual, expected) in decoded.iter().zip(block.iter()) {
                prop_assert!((*actual - *expected).abs() <= 2.0 * scale);
            }
            prop_assert!(decoded.iter().all(|value| !value.is_nan()));
        }

        #[test]
        fn mxfp4_roundtrip_preserves_nonzero_sign(values in proptest::collection::vec(-64.0f32..64.0, MXFP4_BLOCK_SIZE)) {
            let block = finite_block(values);
            let encoded = encode_mxfp4_block(&block).expect("encode");
            let decoded = decode_mxfp4_block(&encoded);
            for (actual, expected) in decoded.iter().zip(block.iter()) {
                if *expected > 0.0 {
                    prop_assert!(*actual > 0.0);
                } else if *expected < 0.0 {
                    prop_assert!(*actual < 0.0);
                } else {
                    prop_assert_eq!(*actual, 0.0);
                }
            }
            prop_assert!(decoded.iter().all(|value| !value.is_nan()));
        }
    }
}
