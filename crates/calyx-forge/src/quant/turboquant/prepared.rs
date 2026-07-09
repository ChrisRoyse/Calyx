use crate::quant::qjl::{qjl_bipolar_mean, read_qjl_section, sign_words};
use crate::quant::{QuantLevel, QuantizedVec};
use crate::{ForgeError, Result};

use super::{TurboQuantCodec, lloyd, packed_len, unpack_codes};

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedQuant {
    pub codes: Vec<u8>,
    pub widths: Vec<u8>,
    pub code_sum: u64,
    pub sign_words: Vec<u64>,
    pub scale: f32,
    pub residual_norm: f32,
    pub level: QuantLevel,
    pub dim: usize,
    pub rot_width: usize,
}

impl PreparedQuant {
    pub(crate) fn qjl_mean(&self, other: &Self) -> f32 {
        qjl_bipolar_mean(&self.sign_words, &other.sign_words, self.rot_width)
    }
}

pub(crate) fn prepare(codec: &TurboQuantCodec, qv: &QuantizedVec) -> Result<PreparedQuant> {
    codec.validate_quantized(qv, "prepare")?;
    let scalar_len = packed_len(codec.rot_width, qv.level);
    let residual = read_qjl_section(&qv.bytes, scalar_len, codec.rot_width)?.ok_or_else(|| {
        quant_error(
            "prepare",
            qv.level,
            "missing QJL residual section; re-encode with TurboQuant v2",
        )
    })?;
    let residual_norm = residual.residual_norm.ok_or_else(|| {
        quant_error(
            "prepare",
            qv.level,
            "legacy QJL v1 section has no residual norm; re-encode with TurboQuant v2",
        )
    })?;
    if residual.rademacher_seed != codec.rademacher().id {
        return Err(quant_error("prepare", qv.level, "rademacher_seed mismatch"));
    }
    let mixed = unpack_codes(&qv.bytes[..scalar_len], codec.rot_width, qv.level);
    let codes = mixed.codes;
    let code_sum = codes.iter().map(|code| u64::from(*code)).sum();
    Ok(PreparedQuant {
        sign_words: sign_words(&residual.bits, codec.rot_width),
        widths: mixed.widths,
        codes,
        code_sum,
        scale: qv.scale,
        residual_norm,
        level: qv.level,
        dim: qv.dim,
        rot_width: codec.rot_width,
    })
}

pub(crate) fn dot_prepared(a: &PreparedQuant, b: &PreparedQuant) -> f32 {
    debug_assert_eq!(a.level, b.level);
    debug_assert_eq!(a.rot_width, b.rot_width);
    if let Some(endpoint) = endpoint_dot(a, b) {
        return endpoint;
    }
    let scalar = scalar_dot(a, b);
    let rho = (std::f32::consts::FRAC_PI_2 * a.qjl_mean(b)).sin();
    scalar + a.residual_norm * b.residual_norm * rho
}

fn endpoint_dot(a: &PreparedQuant, b: &PreparedQuant) -> Option<f32> {
    if !matching_endpoint_shape(a, b) {
        return None;
    }
    if a.codes == b.codes && a.sign_words == b.sign_words {
        return Some(a.scale * b.scale);
    }
    if codes_are_complements(a, b) && sign_words_are_complements(a, b) {
        return Some(-(a.scale * b.scale));
    }
    None
}

fn matching_endpoint_shape(a: &PreparedQuant, b: &PreparedQuant) -> bool {
    a.level == b.level
        && a.dim == b.dim
        && a.rot_width == b.rot_width
        && a.widths == b.widths
        && a.scale.to_bits() == b.scale.to_bits()
        && a.residual_norm.to_bits() == b.residual_norm.to_bits()
}

fn codes_are_complements(a: &PreparedQuant, b: &PreparedQuant) -> bool {
    a.codes
        .iter()
        .zip(a.widths.iter())
        .zip(b.codes.iter())
        .all(|((left, width), right)| {
            let max_code = (1_u16 << *width) - 1;
            u16::from(*left) + u16::from(*right) == max_code
        })
}

fn sign_words_are_complements(a: &PreparedQuant, b: &PreparedQuant) -> bool {
    a.sign_words
        .iter()
        .zip(b.sign_words.iter())
        .enumerate()
        .all(|(idx, (left, right))| {
            let mask = tail_mask(idx, a.rot_width);
            ((left ^ right) & mask) == mask
        })
}

fn tail_mask(word_idx: usize, dim: usize) -> u64 {
    let tail = dim % 64;
    if tail == 0 || word_idx + 1 < dim.div_ceil(64) {
        u64::MAX
    } else {
        (1_u64 << tail) - 1
    }
}

fn scalar_dot(a: &PreparedQuant, b: &PreparedQuant) -> f32 {
    if a.scale == 0.0 || b.scale == 0.0 {
        return 0.0;
    }
    let centroid_dot = lloyd::centroid_product_sum_mixed(&a.codes, &a.widths, &b.codes, &b.widths);
    a.scale * b.scale * centroid_dot / a.rot_width as f32
}

fn quant_error(op: &str, level: QuantLevel, detail: impl Into<String>) -> ForgeError {
    ForgeError::QuantError {
        op: op.to_string(),
        level: format!("{level:?}"),
        detail: detail.into(),
        remediation: "Use matching TurboQuant v2 seeds and encoded QJL residual sections"
            .to_string(),
    }
}
