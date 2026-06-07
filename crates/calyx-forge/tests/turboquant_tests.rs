use calyx_forge::{
    ForgeError, QuantLevel, QuantizedVec, Quantizer, TurboQuantCodec, dot_estimate_unbiased,
    new_seed,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

fn run_cosine_error_trial(level: QuantLevel, dim: usize, n_pairs: usize, seed: u64) -> f32 {
    let codec = TurboQuantCodec::new(new_seed(dim, b"ph14_fsv"), level).expect("codec");
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut total = 0.0_f32;
    for _ in 0..n_pairs {
        let mut left = random_vec(dim, &mut rng);
        let mut right = random_vec(dim, &mut rng);
        normalize_unit(&mut left).expect("left unit vector");
        normalize_unit(&mut right).expect("right unit vector");
        let true_cosine = dot(&left, &right);
        let q_left = codec.encode(&left).expect("encode left");
        let q_right = codec.encode(&right).expect("encode right");
        let estimated = dot_estimate_unbiased(&codec, &q_left, &q_right).expect("dot estimate");
        total += (estimated - true_cosine).abs();
    }
    total / n_pairs as f32
}

fn random_vec(dim: usize, rng: &mut ChaCha8Rng) -> Vec<f32> {
    (0..dim).map(|_| rng.gen_range(-1.0..1.0)).collect()
}

fn normalize_unit(vec: &mut [f32]) -> Result<(), ForgeError> {
    if let Some(idx) = vec.iter().position(|value| !value.is_finite()) {
        return Err(numerical_error(format!(
            "non-finite vector coefficient at index {idx}"
        )));
    }
    let norm = dot(vec, vec).sqrt();
    if norm == 0.0 {
        return Err(numerical_error("zero-norm vector".to_string()));
    }
    for value in vec {
        *value /= norm;
    }
    Ok(())
}

fn cosine(left: &[f32], right: &[f32]) -> Result<f32, ForgeError> {
    if left.len() != right.len() {
        return Err(numerical_error(format!(
            "cosine shape mismatch: left={} right={}",
            left.len(),
            right.len()
        )));
    }
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    normalize_unit(&mut left)?;
    normalize_unit(&mut right)?;
    Ok(dot(&left, &right))
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right.iter()).map(|(a, b)| a * b).sum()
}

fn unit_fixture(dim: usize, seed: u64) -> Vec<f32> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut vec = random_vec(dim, &mut rng);
    normalize_unit(&mut vec).expect("unit fixture");
    vec
}

fn unit_basis(dim: usize, idx: usize) -> Vec<f32> {
    let mut vec = vec![0.0; dim];
    vec[idx] = 1.0;
    vec
}

fn encoded_summary(name: &str, encoded: &QuantizedVec) {
    let first16 = encoded
        .bytes
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("");
    assert!(first16.as_bytes().iter().any(|byte| *byte != b'0'));
    println!(
        "{name} bytes={first16} len={} scale={:.8}",
        encoded.bytes.len(),
        encoded.scale
    );
}

fn numerical_error(detail: String) -> ForgeError {
    ForgeError::NumericalInvariant {
        op: "turboquant_operating_point".to_string(),
        detail,
        remediation: "Use finite non-zero vectors for cosine operating-point FSV".to_string(),
    }
}

#[test]
fn operating_point_bits3p5_dim128() {
    let result = run_cosine_error_trial(QuantLevel::Bits3p5, 128, 1000, 42);
    assert!(result <= 0.05, "{result}");
    let codec =
        TurboQuantCodec::new(new_seed(128, b"ph14_fsv"), QuantLevel::Bits3p5).expect("codec");
    let encoded = codec.encode(&unit_fixture(128, 7)).expect("encode");
    encoded_summary("operating_point_bits3p5_dim128", &encoded);
    println!("operating_point_bits3p5_dim128 PASSED cosine_err_bits3p5={result:.4}");
}

#[test]
fn operating_point_bits2p5_dim128() {
    let result = run_cosine_error_trial(QuantLevel::Bits2p5, 128, 1000, 42);
    assert!(result <= 0.10, "{result}");
    let codec =
        TurboQuantCodec::new(new_seed(128, b"ph14_fsv"), QuantLevel::Bits2p5).expect("codec");
    let encoded = codec.encode(&unit_fixture(128, 8)).expect("encode");
    encoded_summary("operating_point_bits2p5_dim128", &encoded);
    println!("operating_point_bits2p5_dim128 PASSED cosine_err_bits2p5={result:.4}");
}

#[test]
fn operating_point_bits3p5_dim768() {
    let result = run_cosine_error_trial(QuantLevel::Bits3p5, 768, 1000, 42);
    assert!(result <= 0.03, "{result}");
    let codec =
        TurboQuantCodec::new(new_seed(768, b"ph14_fsv"), QuantLevel::Bits3p5).expect("codec");
    let encoded = codec.encode(&unit_fixture(768, 9)).expect("encode");
    encoded_summary("operating_point_bits3p5_dim768", &encoded);
    println!("operating_point_bits3p5_dim768 PASSED cosine_err_bits3p5_dim768={result:.4}");
}

#[test]
fn encode_decode_roundtrip_bits3p5() {
    let codec =
        TurboQuantCodec::new(new_seed(128, b"ph14_roundtrip"), QuantLevel::Bits3p5).expect("codec");
    let original = unit_basis(128, 0);
    let encoded = codec.encode(&original).expect("encode");
    let decoded = codec.decode(&encoded).expect("decode");
    let cosine_loss = 1.0 - cosine(&decoded, &original).expect("cosine");
    assert!(cosine_loss <= 0.01, "{cosine_loss}");
    encoded_summary("encode_decode_roundtrip_bits3p5", &encoded);
    println!("encode_decode_roundtrip_bits3p5 PASSED cosine_loss={cosine_loss:.6}");
}

#[test]
fn encode_decode_roundtrip_bits2p5() {
    let codec =
        TurboQuantCodec::new(new_seed(128, b"ph14_roundtrip"), QuantLevel::Bits2p5).expect("codec");
    let original = unit_basis(128, 0);
    let encoded = codec.encode(&original).expect("encode");
    let decoded = codec.decode(&encoded).expect("decode");
    let cosine_loss = 1.0 - cosine(&decoded, &original).expect("cosine");
    assert!(cosine_loss <= 0.05, "{cosine_loss}");
    encoded_summary("encode_decode_roundtrip_bits2p5", &encoded);
    println!("encode_decode_roundtrip_bits2p5 PASSED cosine_loss={cosine_loss:.6}");
}

#[test]
fn operating_point_edges_single_pair_dim1_and_zero_norm() {
    let single = run_cosine_error_trial(QuantLevel::Bits3p5, 128, 1, 42);
    assert!(single <= 0.10, "{single}");
    let dim1_codec =
        TurboQuantCodec::new(new_seed(1, b"ph14_dim1"), QuantLevel::Bits3p5).expect("codec");
    let dim1_vec = vec![1.0];
    let dim1_q = dim1_codec.encode(&dim1_vec).expect("dim1 encode");
    let dim1_decoded = dim1_codec.decode(&dim1_q).expect("dim1 decode");
    let dim1_loss = 1.0 - cosine(&dim1_decoded, &dim1_vec).expect("dim1 cosine");
    assert!(dim1_loss <= 1e-6, "{dim1_loss}");
    let mut zero = vec![0.0; 128];
    let err = normalize_unit(&mut zero).expect_err("zero-norm must fail closed");
    assert!(matches!(err, ForgeError::NumericalInvariant { .. }));
    println!("operating_point_edges PASSED single_pair={single:.4} dim1_loss={dim1_loss:.6} {err}");
}

#[test]
fn non_finite_encode_fails_closed() {
    let codec =
        TurboQuantCodec::new(new_seed(8, b"ph14_nonfinite"), QuantLevel::Bits3p5).expect("codec");
    let mut vec = unit_fixture(8, 12);
    vec[4] = f32::INFINITY;
    let err = codec.encode(&vec).expect_err("non-finite encode must fail");
    assert!(matches!(err, ForgeError::NumericalInvariant { .. }));
    assert!(
        err.to_string()
            .starts_with("CALYX_FORGE_NUMERICAL_INVARIANT")
    );
    println!("non_finite_encode PASSED {err}");
}

proptest::proptest! {
    #[test]
    fn encoded_seed_id_preserves_rotation_seed(values in proptest::collection::vec(-1.0f32..1.0, 16)) {
        let seed = new_seed(16, b"ph14_prop_seed");
        let codec = TurboQuantCodec::new(seed.clone(), QuantLevel::Bits3p5).expect("codec");
        let mut vec = values;
        if normalize_unit(&mut vec).is_err() {
            vec[0] = 1.0;
        }
        let encoded = codec.encode(&vec).expect("encode");
        proptest::prop_assert_eq!(encoded.seed_id, seed.id);
    }

    #[test]
    fn decode_preserves_dimension(values in proptest::collection::vec(-1.0f32..1.0, 1..96)) {
        let dim = values.len();
        let codec = TurboQuantCodec::new(new_seed(dim, b"ph14_prop_dim"), QuantLevel::Bits2p5)
            .expect("codec");
        let mut vec = values;
        if normalize_unit(&mut vec).is_err() {
            vec[0] = 1.0;
        }
        let decoded = codec.decode(&codec.encode(&vec).expect("encode")).expect("decode");
        proptest::prop_assert_eq!(decoded.len(), dim);
    }
}
