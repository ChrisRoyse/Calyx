use calyx_forge::{
    CURRENT_SEED_VERSION, ForgeError, QuantLevel, QuantizedVec, Quantizer, RotationSeed,
    TurboQuantCodec, dot_estimate_unbiased, new_seed,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::path::PathBuf;

const GOLDEN_SEED_REL: &str = "tests/golden/turboquant_seed_v1.json";

fn run_cosine_error_trial(level: QuantLevel, dim: usize, n_pairs: usize, seed: u64) -> f32 {
    run_cosine_error_trial_with_seed(level, new_seed(dim, b"ph14_fsv"), n_pairs, seed)
}

fn run_cosine_error_trial_with_seed(
    level: QuantLevel,
    rotation_seed: RotationSeed,
    n_pairs: usize,
    seed: u64,
) -> f32 {
    let dim = rotation_seed.dim;
    let codec = TurboQuantCodec::new(rotation_seed, level).expect("codec");
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
    let first16 = first16_hex(&encoded.bytes);
    assert!(first16.as_bytes().iter().any(|byte| *byte != b'0'));
    println!(
        "{name} bytes={first16} len={} scale={:.8}",
        encoded.bytes.len(),
        encoded.scale
    );
}

fn first16_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn replay_seed() -> RotationSeed {
    new_seed(128, b"replay_test_seed")
}

fn golden_seed_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_SEED_REL)
}

fn golden_seed_json() -> String {
    let mut json = serde_json::to_string_pretty(&replay_seed()).expect("golden seed json");
    json.push('\n');
    json
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

#[test]
fn generate_golden_seed() {
    let path = golden_seed_path();
    let expected = golden_seed_json();
    if std::env::var_os("CALYX_WRITE_GOLDEN_SEED").is_some() {
        std::fs::create_dir_all(path.parent().expect("golden seed parent")).expect("mkdir");
        std::fs::write(&path, expected.as_bytes()).expect("write golden seed");
    }
    let actual = std::fs::read_to_string(&path).expect("read committed golden seed");
    assert_eq!(actual.replace("\r\n", "\n"), expected);
    println!(
        "generate_golden_seed PASSED golden_seed_bytes={} path={}",
        actual.len(),
        GOLDEN_SEED_REL
    );
}

#[test]
fn seed_replay_bit_identical() {
    let seed = replay_seed();
    let input = unit_fixture(128, 88);
    let codec1 = TurboQuantCodec::new(seed.clone(), QuantLevel::Bits3p5).expect("codec1");
    let codec2 = TurboQuantCodec::new(seed, QuantLevel::Bits3p5).expect("codec2");
    let first = codec1.encode(&input).expect("first encode");
    let second = codec2.encode(&input).expect("second encode");
    assert_eq!(first.bytes, second.bytes);
    let first_hex = first16_hex(&first.bytes);
    let second_hex = first16_hex(&second.bytes);
    println!(
        "seed_replay_bit_identical PASSED seed_replay_bytes={first_hex} seed_replay_bytes_again={second_hex} len={}",
        first.bytes.len()
    );
}

#[test]
fn seed_id_is_stable_across_reconstruction() {
    let seed = replay_seed();
    let json = serde_json::to_string(&seed).expect("seed json");
    let restored: RotationSeed = serde_json::from_str(&json).expect("seed restore");
    assert_eq!(seed.id, restored.id);
    let input = unit_fixture(128, 89);
    let first = TurboQuantCodec::new(seed, QuantLevel::Bits3p5)
        .expect("codec")
        .encode(&input)
        .expect("first");
    let second = TurboQuantCodec::new(restored, QuantLevel::Bits3p5)
        .expect("codec")
        .encode(&input)
        .expect("second");
    assert_eq!(first.bytes, second.bytes);
    println!(
        "seed_id_is_stable_across_reconstruction PASSED seed_replay_bytes={}",
        first16_hex(&first.bytes)
    );
}

#[test]
fn different_seeds_produce_different_encodings() {
    let input = unit_fixture(128, 90);
    let left = TurboQuantCodec::new(new_seed(128, b"replay_left"), QuantLevel::Bits3p5)
        .expect("left")
        .encode(&input)
        .expect("left encode");
    let right = TurboQuantCodec::new(new_seed(128, b"replay_right"), QuantLevel::Bits3p5)
        .expect("right")
        .encode(&input)
        .expect("right encode");
    assert_ne!(left.bytes, right.bytes);
    let differing = left
        .bytes
        .iter()
        .zip(right.bytes.iter())
        .filter(|(left, right)| left != right)
        .count();
    assert!(differing > 0);
    println!("different_seeds_produce_different_encodings PASSED differing_bytes={differing}");
}

#[test]
fn cosine_error_within_epsilon() {
    let json = std::fs::read_to_string(golden_seed_path()).expect("golden seed");
    let seed: RotationSeed = serde_json::from_str(&json).expect("golden seed json");
    let result = run_cosine_error_trial_with_seed(QuantLevel::Bits3p5, seed, 1000, 42);
    assert!(result <= 0.05, "{result}");
    println!("cosine_error_within_epsilon PASSED cosine_err_seed_replay={result:.4}");
}

#[test]
fn seed_replay_edges_version_json_and_dim768() {
    let mut bumped = replay_seed();
    bumped.version = CURRENT_SEED_VERSION + 1;
    let err = TurboQuantCodec::new(bumped, QuantLevel::Bits3p5).expect_err("version mismatch");
    assert!(matches!(err, ForgeError::SeedVersionMismatch { .. }));

    let seed = replay_seed();
    let json = serde_json::to_string(&seed).expect("seed json");
    let restored: RotationSeed = serde_json::from_str(&json).expect("seed restore");
    assert_eq!(seed_id_bytes(&seed), seed_id_bytes(&restored));

    let input = unit_fixture(768, 91);
    let dim_seed = new_seed(768, b"replay_dim768");
    let first = TurboQuantCodec::new(dim_seed.clone(), QuantLevel::Bits3p5)
        .expect("first")
        .encode(&input)
        .expect("first encode");
    let second = TurboQuantCodec::new(dim_seed, QuantLevel::Bits3p5)
        .expect("second")
        .encode(&input)
        .expect("second encode");
    assert_eq!(first.bytes, second.bytes);
    println!(
        "seed_replay_edges PASSED dim768_bytes={} seed_version_error={err}",
        first16_hex(&first.bytes)
    );
}

fn seed_id_bytes(seed: &RotationSeed) -> [u8; 32] {
    seed.id
}

#[test]
fn decode_with_wrong_seed_fails_closed() {
    let input = unit_fixture(128, 92);
    let right_seed = new_seed(128, b"wrong_seed_right");
    let encoded = TurboQuantCodec::new(new_seed(128, b"wrong_seed_left"), QuantLevel::Bits3p5)
        .expect("left")
        .encode(&input)
        .expect("encode");
    let err = TurboQuantCodec::new(right_seed, QuantLevel::Bits3p5)
        .expect("right")
        .decode(&encoded)
        .expect_err("wrong seed must fail");
    assert!(matches!(err, ForgeError::QuantError { .. }));
    assert!(err.to_string().contains("seed_id mismatch"));
    println!("decode_with_wrong_seed_fails_closed PASSED {err}");
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

    #[test]
    fn replay_encoded_length_stable_across_repeated_encodes(values in proptest::collection::vec(-1.0f32..1.0, 16)) {
        let seed = new_seed(16, b"ph14_replay_len");
        let codec = TurboQuantCodec::new(seed, QuantLevel::Bits3p5).expect("codec");
        let mut vec = values;
        if normalize_unit(&mut vec).is_err() {
            vec[0] = 1.0;
        }
        let first = codec.encode(&vec).expect("first");
        for _ in 0..100 {
            let replay = codec.encode(&vec).expect("replay");
            proptest::prop_assert_eq!(replay.bytes.len(), first.bytes.len());
            proptest::prop_assert_eq!(&replay.bytes, &first.bytes);
        }
    }

    #[test]
    fn different_inputs_change_encoded_bytes(
        left_idx in 0usize..16,
        offset in 1usize..16,
        left_positive in proptest::bool::ANY,
        right_positive in proptest::bool::ANY
    ) {
        let right_idx = (left_idx + offset) % 16;
        let mut left = vec![0.0; 16];
        let mut right = vec![0.0; 16];
        left[left_idx] = if left_positive { 1.0 } else { -1.0 };
        right[right_idx] = if right_positive { 1.0 } else { -1.0 };
        let codec = TurboQuantCodec::new(new_seed(16, b"ph14_replay_inputs"), QuantLevel::Bits3p5)
            .expect("codec");
        let left_encoded = codec.encode(&left).expect("left encode");
        let right_encoded = codec.encode(&right).expect("right encode");
        proptest::prop_assert_ne!(left_encoded.bytes, right_encoded.bytes);
    }
}
