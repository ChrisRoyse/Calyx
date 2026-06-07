use super::*;
use crate::quant::new_seed;
use proptest::prelude::*;

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

fn encoded_len(dim: usize, level: QuantLevel) -> usize {
    packed_len(dim, level) + 1 + 32 + dim.div_ceil(8)
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
    assert_eq!(qv.bytes.len(), encoded_len(128, QuantLevel::Bits3p5));
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
    let qv = codec.encode(&input).expect("encode");
    let decoded = codec.decode(&qv).expect("decode");
    let max_err = max_abs_delta(&decoded, &input);
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
    assert_eq!(first.bytes.len(), encoded_len(128, QuantLevel::Bits3p5));
    assert_eq!(low.bytes.len(), encoded_len(128, QuantLevel::Bits2p5));
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
    assert!(max_abs_delta(&one_decoded, &[2.0]) <= 1e-6);

    let large_seed = new_seed(1536, b"tq_large");
    let large_codec = TurboQuantCodec::new(large_seed, QuantLevel::Bits3p5).expect("large codec");
    let large_qv = large_codec.encode(&vec![0.0; 1536]).expect("large encode");
    let large_decoded = large_codec.decode(&large_qv).expect("large decode");
    assert!(large_decoded.iter().all(|value| value.is_finite()));
    assert_eq!(large_qv.bytes.len(), encoded_len(1536, QuantLevel::Bits3p5));

    let same_seed = new_seed(128, b"tq_identical");
    let same_codec = TurboQuantCodec::new(same_seed.clone(), QuantLevel::Bits2p5).expect("same");
    let same_vec = vec![0.25; 128];
    let same_qv = same_codec.encode(&same_vec).expect("same encode");
    let same_decoded = same_codec.decode(&same_qv).expect("same decode");
    let same_err = max_abs_delta(&same_decoded, &same_vec);
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
    assert!(matches!(err, ForgeError::NumericalInvariant { .. }));
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
        let qv = codec.encode(&values).expect("encode");
        let decoded = codec.decode(&qv).expect("decode");
        let max_err = max_abs_delta(&decoded, &values);
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
        prop_assert_eq!(left_qv.bytes.len(), encoded_len(dim, level));
    }
}
