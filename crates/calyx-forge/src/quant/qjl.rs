use crate::quant::{QuantLevel, QuantizedVec, RotationSeed, SeedId, TurboQuantCodec};
use crate::{ForgeError, Result};

pub const QJL_SECTION_TAG_V1: u8 = 0x01;
pub const QJL_SECTION_TAG: u8 = 0x02;

#[derive(Clone, Debug, PartialEq)]
pub struct QjlResidual {
    pub bits: Vec<u8>,
    pub rademacher_seed: SeedId,
    pub residual_norm: Option<f32>,
}

pub fn encode_qjl_residual(
    rotated: &[f32],
    scalar_decoded: &[f32],
    rademacher: &RotationSeed,
) -> QjlResidual {
    assert_eq!(
        rotated.len(),
        scalar_decoded.len(),
        "QJL residual dimension mismatch: rotated={} scalar_decoded={}",
        rotated.len(),
        scalar_decoded.len()
    );
    assert_eq!(
        rotated.len(),
        rademacher.dim,
        "QJL rademacher dimension mismatch: expected {} got {}",
        rademacher.dim,
        rotated.len()
    );
    let mut residual = rotated
        .iter()
        .zip(scalar_decoded.iter())
        .map(|(rotated, decoded)| rotated - decoded)
        .collect::<Vec<_>>();
    let residual_norm = residual
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    crate::quant::apply_rotation(rademacher, &mut residual);
    let mut bits = vec![0; qjl_bits_len(rotated.len())];
    for (idx, residual) in residual.iter().enumerate() {
        if *residual > 0.0 {
            bits[idx / 8] |= 1 << (idx % 8);
        }
    }
    QjlResidual {
        bits,
        rademacher_seed: rademacher.id,
        residual_norm: Some(residual_norm),
    }
}

pub fn dot_qjl_correction(
    qa: &QjlResidual,
    qb: &QjlResidual,
    rademacher: &RotationSeed,
    scale_a: f32,
    scale_b: f32,
) -> f32 {
    assert_eq!(
        qa.rademacher_seed, rademacher.id,
        "QJL rademacher seed mismatch"
    );
    assert_eq!(
        qb.rademacher_seed, rademacher.id,
        "QJL rademacher seed mismatch"
    );
    assert_eq!(
        qa.bits.len(),
        qjl_bits_len(rademacher.dim),
        "QJL bit length mismatch"
    );
    assert_eq!(
        qb.bits.len(),
        qjl_bits_len(rademacher.dim),
        "QJL bit length mismatch"
    );
    if rademacher.dim == 0 {
        return 0.0;
    }
    let mean = qjl_bipolar_mean(
        &sign_words(&qa.bits, rademacher.dim),
        &sign_words(&qb.bits, rademacher.dim),
        rademacher.dim,
    );
    match (qa.residual_norm, qb.residual_norm) {
        (Some(norm_a), Some(norm_b)) => {
            norm_a * norm_b * (std::f32::consts::FRAC_PI_2 * mean).sin()
        }
        _ => scale_a * scale_b * mean,
    }
}

pub fn dot_estimate_unbiased(
    codec: &TurboQuantCodec,
    qv_a: &QuantizedVec,
    qv_b: &QuantizedVec,
) -> Result<f32> {
    let a = codec.prepare(qv_a)?;
    let b = codec.prepare(qv_b)?;
    Ok(codec.dot_prepared(&a, &b))
}

pub(crate) fn append_qjl_section(bytes: &mut Vec<u8>, residual: &QjlResidual) {
    bytes.push(QJL_SECTION_TAG);
    bytes.extend_from_slice(&residual.rademacher_seed);
    let residual_norm = residual.residual_norm.unwrap_or(0.0);
    bytes.extend_from_slice(&residual_norm.to_le_bytes());
    bytes.extend_from_slice(&residual.bits);
}

pub(crate) fn read_qjl_section(
    bytes: &[u8],
    scalar_len: usize,
    dim: usize,
) -> Result<Option<QjlResidual>> {
    if bytes.len() == scalar_len {
        return Ok(None);
    }
    let tag = bytes[scalar_len];
    let (expected_total, residual_norm_len) = match tag {
        QJL_SECTION_TAG_V1 => (scalar_len + qjl_section_len_v1(dim), 0),
        QJL_SECTION_TAG => (scalar_len + qjl_section_len(dim), 4),
        _ => {
            return Err(qjl_error(
                "decode",
                QuantLevel::Bits3p5,
                "missing QJL section tag",
            ));
        }
    };
    if bytes.len() != expected_total {
        return Err(qjl_error(
            "decode",
            QuantLevel::Bits3p5,
            format!(
                "QJL section length mismatch: expected {expected_total} got {}",
                bytes.len()
            ),
        ));
    }
    let seed_start = scalar_len + 1;
    let norm_start = seed_start + 32;
    let bits_start = norm_start + residual_norm_len;
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&bytes[seed_start..norm_start]);
    let residual_norm = if residual_norm_len == 0 {
        None
    } else {
        let norm = f32::from_le_bytes(bytes[norm_start..bits_start].try_into().unwrap());
        if !norm.is_finite() || norm < 0.0 {
            return Err(qjl_error(
                "decode",
                QuantLevel::Bits3p5,
                "QJL residual norm must be finite and non-negative",
            ));
        }
        Some(norm)
    };
    Ok(Some(QjlResidual {
        bits: bytes[bits_start..].to_vec(),
        rademacher_seed: seed,
        residual_norm,
    }))
}

pub(crate) fn qjl_bits_len(dim: usize) -> usize {
    dim.div_ceil(8)
}

fn qjl_section_len(dim: usize) -> usize {
    1 + 32 + 4 + qjl_bits_len(dim)
}

fn qjl_section_len_v1(dim: usize) -> usize {
    1 + 32 + qjl_bits_len(dim)
}

pub(crate) fn sign_words(bits: &[u8], dim: usize) -> Vec<u64> {
    let mut words = vec![0_u64; dim.div_ceil(64)];
    for idx in 0..dim {
        if ((bits[idx / 8] >> (idx % 8)) & 1) == 1 {
            words[idx / 64] |= 1_u64 << (idx % 64);
        }
    }
    words
}

pub(crate) fn qjl_bipolar_mean(left: &[u64], right: &[u64], dim: usize) -> f32 {
    debug_assert_eq!(left.len(), dim.div_ceil(64));
    debug_assert_eq!(right.len(), dim.div_ceil(64));
    if dim == 0 {
        return 0.0;
    }
    let mut mismatches = 0_u64;
    for (idx, (left, right)) in left.iter().zip(right.iter()).enumerate() {
        let mut xor = left ^ right;
        if idx + 1 == dim.div_ceil(64) {
            let tail = dim % 64;
            if tail != 0 {
                xor &= (1_u64 << tail) - 1;
            }
        }
        mismatches += u64::from(xor.count_ones());
    }
    let matches_minus_mismatches = dim as f32 - 2.0 * mismatches as f32;
    matches_minus_mismatches / dim as f32
}

fn qjl_error(op: &str, level: QuantLevel, detail: impl Into<String>) -> ForgeError {
    ForgeError::QuantError {
        op: op.to_string(),
        level: format!("{level:?}"),
        detail: detail.into(),
        remediation: "Use matching quantizer seeds and encoded QJL residual sections".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quant::{Quantizer, new_seed};
    use rand::{Rng, SeedableRng, rngs::StdRng};

    fn unit_basis(dim: usize, idx: usize) -> Vec<f32> {
        let mut vec = vec![0.0; dim];
        vec[idx] = 1.0;
        vec
    }

    fn random_unit_pair(rng: &mut StdRng, dim: usize) -> (Vec<f32>, Vec<f32>) {
        let mut left = vec![0.0; dim];
        let mut right = vec![0.0; dim];
        for idx in 0..dim {
            left[idx] = rng.gen_range(-1.0..1.0);
            right[idx] = rng.gen_range(-1.0..1.0);
        }
        normalize(&mut left);
        normalize(&mut right);
        (left, right)
    }

    fn normalize(vec: &mut [f32]) {
        let norm = vec
            .iter()
            .map(|value| f64::from(*value) * f64::from(*value))
            .sum::<f64>()
            .sqrt() as f32;
        if norm == 0.0 {
            vec[0] = 1.0;
        } else {
            for value in vec {
                *value /= norm;
            }
        }
    }

    fn dot(left: &[f32], right: &[f32]) -> f32 {
        left.iter().zip(right.iter()).map(|(a, b)| a * b).sum()
    }

    fn mean_abs_error(level: QuantLevel) -> f32 {
        let codec = TurboQuantCodec::new(new_seed(128, b"qjl_mean_seed_42"), level).expect("codec");
        let mut rng = StdRng::seed_from_u64(42);
        let mut err = 0.0_f32;
        for _ in 0..1000 {
            let (left, right) = random_unit_pair(&mut rng, 128);
            let q_left = codec.encode(&left).expect("encode left");
            let q_right = codec.encode(&right).expect("encode right");
            let estimate = dot_estimate_unbiased(&codec, &q_left, &q_right).expect("dot");
            err += (estimate - dot(&left, &right)).abs();
        }
        err / 1000.0
    }

    fn mean_signed_error(level: QuantLevel) -> f32 {
        let codec =
            TurboQuantCodec::new(new_seed(128, b"qjl_signed_seed_42"), level).expect("codec");
        let mut rng = StdRng::seed_from_u64(4242);
        let mut err = 0.0_f32;
        for _ in 0..2000 {
            let (left, right) = random_unit_pair(&mut rng, 128);
            let q_left = codec.encode(&left).expect("encode left");
            let q_right = codec.encode(&right).expect("encode right");
            let estimate = dot_estimate_unbiased(&codec, &q_left, &q_right).expect("dot");
            err += estimate - dot(&left, &right);
        }
        err / 2000.0
    }

    #[test]
    fn unbiased_dot_self_and_orthogonal_bits3p5() {
        let codec = TurboQuantCodec::new(new_seed(128, b"qjl_self_orth"), QuantLevel::Bits3p5)
            .expect("codec");
        let self_vec = unit_basis(128, 0);
        let orth_vec = unit_basis(128, 1);
        let q_self = codec.encode(&self_vec).expect("self encode");
        let self_est = dot_estimate_unbiased(&codec, &q_self, &q_self).expect("self dot");
        let q_orth = codec.encode(&orth_vec).expect("orth encode");
        let orth_est = dot_estimate_unbiased(&codec, &q_self, &q_orth).expect("orth dot");
        assert!((self_est - 1.0).abs() <= 0.05, "{self_est}");
        assert!(orth_est.is_finite() && orth_est.abs() <= 1.0, "{orth_est}");
        println!("unbiased_dot_self_orthogonal PASSED self={self_est:.6} orth={orth_est:.6}");
    }

    #[test]
    fn unbiased_dot_mean_error_bits3p5() {
        let mean_err = mean_abs_error(QuantLevel::Bits3p5);
        assert!(mean_err <= 0.05, "{mean_err}");
        println!("unbiased_dot_mean_error_bits3p5 PASSED mean_err={mean_err:.6}");
    }

    #[test]
    fn unbiased_dot_mean_error_bits2p5() {
        let mean_err = mean_abs_error(QuantLevel::Bits2p5);
        assert!(mean_err <= 0.10, "{mean_err}");
        println!("unbiased_dot_mean_error_bits2p5 PASSED mean_err={mean_err:.6}");
    }

    #[test]
    fn unbiased_dot_mean_signed_error_is_near_zero() {
        let bits3 = mean_signed_error(QuantLevel::Bits3p5);
        let bits2 = mean_signed_error(QuantLevel::Bits2p5);
        assert!(bits3.abs() <= 0.01, "{bits3}");
        assert!(bits2.abs() <= 0.02, "{bits2}");
        println!("unbiased_dot_mean_signed_error PASSED bits3={bits3:.6} bits2={bits2:.6}");
    }

    #[test]
    fn unbiased_dot_edges_parallel_antiparallel_and_seed_mismatch() {
        let codec =
            TurboQuantCodec::new(new_seed(128, b"qjl_edges"), QuantLevel::Bits3p5).expect("codec");
        let left = unit_basis(128, 3);
        let mut anti = left.clone();
        for value in &mut anti {
            *value = -*value;
        }
        let q_left = codec.encode(&left).expect("left encode");
        let q_parallel = codec.encode(&left).expect("parallel encode");
        let q_anti = codec.encode(&anti).expect("anti encode");
        let parallel = dot_estimate_unbiased(&codec, &q_left, &q_parallel).expect("parallel");
        let antiparallel = dot_estimate_unbiased(&codec, &q_left, &q_anti).expect("anti");
        assert!((parallel - 1.0).abs() <= 0.05, "{parallel}");
        assert!((antiparallel + 1.0).abs() <= 0.05, "{antiparallel}");

        let other =
            TurboQuantCodec::new(new_seed(128, b"qjl_other"), QuantLevel::Bits3p5).expect("other");
        let q_other = other.encode(&left).expect("other encode");
        let err = dot_estimate_unbiased(&codec, &q_left, &q_other)
            .expect_err("seed mismatch must fail closed");
        assert!(err.to_string().contains("seed_id mismatch"));
        println!(
            "unbiased_dot_edges PASSED parallel={parallel:.6} antiparallel={antiparallel:.6} {err}"
        );
    }
}
