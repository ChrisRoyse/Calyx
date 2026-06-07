use super::*;
use crate::quant::{CURRENT_SEED_VERSION, SeedId, new_seed};
use proptest::prelude::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

fn fixed_seed(dim: usize, id_byte: u8) -> RotationSeed {
    RotationSeed {
        id: [id_byte; 32],
        version: CURRENT_SEED_VERSION,
        dim,
        diagonal: vec![1.0; dim],
    }
}

fn seed_with_id(dim: usize, id: SeedId) -> RotationSeed {
    RotationSeed {
        id,
        version: CURRENT_SEED_VERSION,
        dim,
        diagonal: vec![1.0; dim],
    }
}

fn unit_basis(dim: usize, idx: usize) -> Vec<f32> {
    let mut vec = vec![0.0; dim];
    vec[idx] = 1.0;
    vec
}

fn negate(vec: &[f32]) -> Vec<f32> {
    vec.iter().map(|value| -*value).collect()
}

fn first_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right.iter()).map(|(a, b)| a * b).sum()
}

fn normalize(vec: &mut [f32]) {
    let norm = dot(vec, vec).sqrt();
    if norm == 0.0 {
        vec[0] = 1.0;
        return;
    }
    for value in vec {
        *value /= norm;
    }
}

fn random_unit(dim: usize, rng: &mut ChaCha8Rng) -> Vec<f32> {
    let mut vec = (0..dim)
        .map(|_| rng.gen_range(-1.0..1.0))
        .collect::<Vec<_>>();
    normalize(&mut vec);
    vec
}

fn noisy_neighbor(query: &[f32], rng: &mut ChaCha8Rng, noise_scale: f32) -> Vec<f32> {
    let mut vec = query
        .iter()
        .map(|value| *value + rng.gen_range(-noise_scale..noise_scale))
        .collect::<Vec<_>>();
    normalize(&mut vec);
    vec
}

fn true_top_indices(query: &[f32], candidates: &[Vec<f32>], keep: usize) -> Vec<usize> {
    let mut scored = candidates
        .iter()
        .enumerate()
        .map(|(idx, candidate)| (idx, dot(query, candidate)))
        .collect::<Vec<_>>();
    scored.sort_by(|(left_idx, left_score), (right_idx, right_score)| {
        right_score
            .total_cmp(left_score)
            .then_with(|| left_idx.cmp(right_idx))
    });
    scored.into_iter().take(keep).map(|(idx, _)| idx).collect()
}

fn recall_trial(seed: u64) -> f32 {
    let dim = 128;
    let keep = 8;
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let codec = BinaryCodec::new(new_seed(dim, b"binary_prefilter_recall")).expect("codec");
    let query = random_unit(dim, &mut rng);
    let mut candidates = Vec::new();
    for _ in 0..keep {
        candidates.push(noisy_neighbor(&query, &mut rng, 0.015));
    }
    for _ in 0..48 {
        candidates.push(random_unit(dim, &mut rng));
    }
    let query_encoded = codec.encode(&query).expect("query encode");
    let encoded_candidates = candidates
        .iter()
        .map(|candidate| codec.encode(candidate).expect("candidate encode"))
        .collect::<Vec<_>>();
    let selected = binary_prefilter(&query_encoded, &encoded_candidates, keep).expect("prefilter");
    let true_top = true_top_indices(&query, &candidates, keep);
    let hits = selected.iter().filter(|idx| true_top.contains(idx)).count();
    hits as f32 / keep as f32
}

#[test]
fn binary_encode_fixed_unit_vector_packs_expected_bits() {
    let codec = BinaryCodec::new(fixed_seed(4, 0x11)).expect("codec");
    let encoded = codec.encode(&unit_basis(4, 0)).expect("encode");
    assert_eq!(encoded.level, QuantLevel::Bits1);
    assert_eq!(encoded.bytes, vec![0x0f]);
    assert_eq!(encoded.bytes.len(), 1);
    println!(
        "binary_encode_fixed_unit_vector PASSED bytes={} len={}",
        first_hex(&encoded.bytes),
        encoded.bytes.len()
    );
}

#[test]
fn binary_hamming_self_is_one() {
    let codec = BinaryCodec::new(fixed_seed(8, 0x22)).expect("codec");
    let encoded = codec.encode(&unit_basis(8, 0)).expect("encode");
    let hamming = hamming_dot_estimate(&encoded, &encoded).expect("hamming");
    assert_eq!(hamming, 1.0);
    println!(
        "binary_hamming_self_is_one PASSED hamming={hamming:.6} bytes={}",
        first_hex(&encoded.bytes)
    );
}

#[test]
fn binary_hamming_negation_is_minus_one() {
    let codec = BinaryCodec::new(fixed_seed(8, 0x33)).expect("codec");
    let vec = unit_basis(8, 0);
    let encoded = codec.encode(&vec).expect("encode");
    let negated = codec.encode(&negate(&vec)).expect("encode negated");
    let hamming = hamming_dot_estimate(&encoded, &negated).expect("hamming");
    assert_eq!(hamming, -1.0);
    println!("binary_hamming_negation PASSED hamming={hamming:.6}");
}

#[test]
fn binary_decode_returns_coarse_unit_direction() {
    let codec = BinaryCodec::new(fixed_seed(4, 0x44)).expect("codec");
    let decoded = codec
        .decode(&codec.encode(&unit_basis(4, 0)).expect("encode"))
        .expect("decode");
    assert_eq!(decoded.len(), 4);
    assert!((dot(&decoded, &decoded) - 1.0).abs() <= 1e-6);
    assert!((decoded[0] - 1.0).abs() <= 1e-6);
    println!(
        "binary_decode_returns_coarse_unit_direction PASSED norm={:.6}",
        dot(&decoded, &decoded).sqrt()
    );
}

#[test]
fn binary_edges_dim1_keep_all_and_partial_byte() {
    let one = BinaryCodec::new(fixed_seed(1, 0x55)).expect("dim1 codec");
    let encoded_one = one.encode(&[1.0]).expect("dim1 encode");
    assert_eq!(encoded_one.bytes, vec![0x01]);
    assert_eq!(one.decode(&encoded_one).expect("dim1 decode"), vec![1.0]);

    let partial = BinaryCodec::new(fixed_seed(10, 0x66)).expect("partial codec");
    let encoded_partial = partial.encode(&unit_basis(10, 0)).expect("partial encode");
    assert_eq!(encoded_partial.bytes, vec![0xff, 0x00]);

    let query = one.encode(&[1.0]).expect("query");
    let candidates = vec![
        one.encode(&[1.0]).expect("candidate same"),
        one.encode(&[-1.0]).expect("candidate opposite"),
    ];
    assert_eq!(
        binary_prefilter(&query, &candidates, 99).expect("keep all"),
        vec![0, 1]
    );
    println!(
        "binary_edges PASSED dim1_bytes={} partial_bytes={} keep_all=2",
        first_hex(&encoded_one.bytes),
        first_hex(&encoded_partial.bytes)
    );
}

#[test]
fn binary_fail_closed_nonfinite_seed_mismatch_and_padding() {
    let codec = BinaryCodec::new(fixed_seed(8, 0x77)).expect("codec");
    let err = codec
        .encode(&[1.0, f32::INFINITY, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
        .expect_err("non-finite input must fail");
    assert!(matches!(err, ForgeError::NumericalInvariant { .. }));

    let left = BinaryCodec::new(seed_with_id(8, [1; 32]))
        .expect("left")
        .encode(&unit_basis(8, 0))
        .expect("left encode");
    let right = BinaryCodec::new(seed_with_id(8, [2; 32]))
        .expect("right")
        .encode(&unit_basis(8, 0))
        .expect("right encode");
    let seed_err = hamming_dot_estimate(&left, &right).expect_err("seed mismatch must fail");
    assert!(
        seed_err
            .to_string()
            .contains("seed_id mismatch in hamming_dot_estimate")
    );

    let partial = BinaryCodec::new(fixed_seed(10, 0x78)).expect("partial codec");
    let mut padded = partial.encode(&unit_basis(10, 0)).expect("padded encode");
    padded.bytes[1] |= 0x80;
    let padding_err = partial.decode(&padded).expect_err("padding must fail");
    assert!(padding_err.to_string().contains("non-zero padding bits"));
    println!("binary_fail_closed PASSED {err} {seed_err} {padding_err}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn binary_prefilter_recall(seed in any::<u64>()) {
        let recall = recall_trial(seed);
        println!("binary_prefilter_recall PASSED recall={recall:.2} seed={seed}");
        prop_assert!(recall >= 0.80, "recall={recall}");
    }
}
