use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::quant::SeedId;
use crate::{ForgeError, Result};

pub const CURRENT_SEED_VERSION: u8 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RotationSeed {
    #[serde(with = "seed_id_hex_serde")]
    pub id: SeedId,
    pub version: u8,
    pub dim: usize,
    pub diagonal: Vec<f32>,
}

impl RotationSeed {
    pub fn verify_current_version(&self) -> Result<()> {
        if self.version == CURRENT_SEED_VERSION {
            return Ok(());
        }
        Err(ForgeError::SeedVersionMismatch {
            expected: CURRENT_SEED_VERSION,
            got: self.version,
        })
    }
}

pub fn new_seed(dim: usize, entropy: &[u8]) -> RotationSeed {
    let rng_seed = sha256_entropy_dim(entropy, dim);
    let mut rng = ChaCha8Rng::from_seed(rng_seed);
    let diagonal: Vec<f32> = (0..dim)
        .map(|_| if rng.r#gen::<bool>() { 1.0 } else { -1.0 })
        .collect();
    let id = content_id(&diagonal, CURRENT_SEED_VERSION, dim);
    RotationSeed {
        id,
        version: CURRENT_SEED_VERSION,
        dim,
        diagonal,
    }
}

pub fn apply_rotation(seed: &RotationSeed, vec: &mut [f32]) {
    assert_eq!(
        vec.len(),
        seed.dim,
        "dimension mismatch: expected {} got {}",
        seed.dim,
        vec.len()
    );
    apply_block_hadamard(vec);
    for (value, sign) in vec.iter_mut().zip(seed.diagonal.iter()) {
        *value *= *sign;
    }
}

pub fn apply_inverse_rotation(seed: &RotationSeed, vec: &mut [f32]) {
    assert_eq!(
        vec.len(),
        seed.dim,
        "dimension mismatch: expected {} got {}",
        seed.dim,
        vec.len()
    );
    for (value, sign) in vec.iter_mut().zip(seed.diagonal.iter()) {
        *value *= *sign;
    }
    apply_block_hadamard(vec);
}

pub fn apply_rotation_batch(seed: &RotationSeed, vecs: &mut [f32], n: usize) {
    let expected = n
        .checked_mul(seed.dim)
        .expect("batch dimension mismatch: n * dim overflow");
    assert_eq!(
        vecs.len(),
        expected,
        "batch dimension mismatch: expected {} got {}",
        expected,
        vecs.len()
    );
    for row in vecs.chunks_exact_mut(seed.dim) {
        apply_rotation(seed, row);
    }
}

pub fn seed_id_hex(id: &SeedId) -> String {
    let mut hex = String::with_capacity(id.len() * 2);
    for byte in id {
        hex.push(nibble_hex(byte >> 4));
        hex.push(nibble_hex(byte & 0x0f));
    }
    hex
}

fn sha256_entropy_dim(entropy: &[u8], dim: usize) -> SeedId {
    let mut hasher = Sha256::new();
    hasher.update(entropy);
    hasher.update((dim as u64).to_le_bytes());
    hasher.finalize().into()
}

fn content_id(diagonal: &[f32], version: u8, dim: usize) -> SeedId {
    let mut hasher = Sha256::new();
    for sign in diagonal {
        hasher.update(sign.to_le_bytes());
    }
    hasher.update([version]);
    hasher.update((dim as u64).to_le_bytes());
    hasher.finalize().into()
}

fn apply_block_hadamard(vec: &mut [f32]) {
    let mut offset = 0;
    while offset < vec.len() {
        let block_len = largest_power_of_two_at_most(vec.len() - offset);
        hadamard_power_of_two(&mut vec[offset..offset + block_len]);
        offset += block_len;
    }
}

fn hadamard_power_of_two(block: &mut [f32]) {
    let mut width = 1;
    while width < block.len() {
        let step = width * 2;
        for base in (0..block.len()).step_by(step) {
            for idx in 0..width {
                let left = block[base + idx];
                let right = block[base + idx + width];
                block[base + idx] = left + right;
                block[base + idx + width] = left - right;
            }
        }
        width = step;
    }
    let scale = 1.0 / (block.len() as f32).sqrt();
    for value in block {
        *value *= scale;
    }
}

fn largest_power_of_two_at_most(value: usize) -> usize {
    1usize << (usize::BITS - 1 - value.leading_zeros())
}

fn nibble_hex(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + (nibble - 10)) as char,
        _ => unreachable!("nibble is masked"),
    }
}

fn decode_hex_seed_id(text: &str) -> std::result::Result<SeedId, String> {
    if text.len() != 64 {
        return Err(format!("seed id hex length must be 64, got {}", text.len()));
    }
    let mut id = [0u8; 32];
    for (idx, slot) in id.iter_mut().enumerate() {
        let hi = hex_value(text.as_bytes()[idx * 2])?;
        let lo = hex_value(text.as_bytes()[idx * 2 + 1])?;
        *slot = (hi << 4) | lo;
    }
    Ok(id)
}

fn hex_value(byte: u8) -> std::result::Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(format!("invalid seed id hex byte: {byte}")),
    }
}

mod seed_id_hex_serde {
    use super::*;

    pub fn serialize<S>(id: &SeedId, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&seed_id_hex(id))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<SeedId, D::Error>
    where
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        decode_hex_seed_id(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::time::Instant;

    fn l2_norm(vec: &[f32]) -> f32 {
        vec.iter()
            .map(|value| (*value as f64) * (*value as f64))
            .sum::<f64>()
            .sqrt() as f32
    }

    #[test]
    fn rotation_seed_deterministic() {
        let first = new_seed(128, b"test_entropy_1");
        let second = new_seed(128, b"test_entropy_1");
        assert_eq!(first.id, second.id);
        assert_eq!(first.diagonal, second.diagonal);
        let first_hex = seed_id_hex(&first.id);
        let second_hex = seed_id_hex(&second.id);
        println!(
            "rotation_seed_deterministic PASSED id={} id_again={}",
            &first_hex[..8],
            &second_hex[..8]
        );
    }

    #[test]
    fn rotation_isometric_dim4_basis_vector() {
        let seed = new_seed(4, b"dim4_basis");
        let mut vec = vec![1.0, 0.0, 0.0, 0.0];
        apply_rotation(&seed, &mut vec);
        let norm = l2_norm(&vec);
        println!("rotation_isometric PASSED norm={norm:.6} vec={vec:?}");
        assert!((norm - 1.0).abs() <= 1e-5, "{norm}");
    }

    #[test]
    fn rotation_edges_dim1_dim768_and_version_mismatch() {
        let one = new_seed(1, b"dim1");
        assert_eq!(one.diagonal.len(), 1);
        let mut one_vec = vec![3.0];
        apply_rotation(&one, &mut one_vec);
        assert_eq!(one_vec[0].abs(), 3.0);

        let large = new_seed(768, b"dim768");
        let mut large_vec = vec![0.25; 768];
        let start = Instant::now();
        apply_rotation(&large, &mut large_vec);
        let elapsed_us = start.elapsed().as_micros();
        println!(
            "rotation_dim768 PASSED micros={elapsed_us} norm={:.6}",
            l2_norm(&large_vec)
        );
        assert!(elapsed_us < 1_000, "{elapsed_us}");

        let mut mismatched = one.clone();
        mismatched.version = CURRENT_SEED_VERSION + 1;
        let err = mismatched
            .verify_current_version()
            .expect_err("seed version mismatch must fail closed");
        println!("rotation_version_mismatch PASSED {err}");
        assert!(matches!(err, ForgeError::SeedVersionMismatch { .. }));
    }

    #[test]
    fn rotation_seed_id_serializes_as_hex_string() {
        let seed = new_seed(8, b"serde_seed");
        let json = serde_json::to_string(&seed).expect("rotation seed serialize");
        assert!(json.contains("\"id\":\""));
        let restored: RotationSeed =
            serde_json::from_str(&json).expect("rotation seed deserialize");
        assert_eq!(seed, restored);
        println!(
            "rotation_seed_serde PASSED id={}",
            &seed_id_hex(&restored.id)[..8]
        );
    }

    #[test]
    fn rotation_dimension_mismatch_panics() {
        let seed = new_seed(4, b"mismatch");
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let panic = std::panic::catch_unwind(|| {
            let mut vec = vec![1.0, 2.0, 3.0];
            apply_rotation(&seed, &mut vec);
        });
        std::panic::set_hook(hook);
        let message = panic_message(panic.expect_err("dimension mismatch must panic"));
        assert!(
            message.contains("dimension mismatch: expected 4 got 3"),
            "{message}"
        );
        println!("rotation_dimension_mismatch PASSED");
    }

    #[test]
    fn inverse_rotation_roundtrips_dim8() {
        let seed = new_seed(8, b"inverse");
        let original = vec![1.0, -0.5, 0.25, 0.0, 0.75, -1.25, 0.5, -0.125];
        let mut rotated = original.clone();
        apply_rotation(&seed, &mut rotated);
        apply_inverse_rotation(&seed, &mut rotated);
        let max_err = original
            .iter()
            .zip(rotated.iter())
            .map(|(left, right)| (left - right).abs())
            .fold(0.0_f32, f32::max);
        assert!(max_err <= 1e-6, "{max_err}");
        println!("inverse_rotation_roundtrip PASSED max_err={max_err:.8}");
    }

    proptest! {
        #[test]
        fn rotation_preserves_l2_norm_dim32(values in proptest::collection::vec(-10.0f32..10.0, 32)) {
            let seed = new_seed(32, b"norm_prop");
            let before = l2_norm(&values);
            let mut rotated = values;
            apply_rotation(&seed, &mut rotated);
            let after = l2_norm(&rotated);
            prop_assert!((before - after).abs() <= 1e-5, "before={before} after={after}");
        }

        #[test]
        fn rotation_seed_changes_for_distinct_entropy(
            (left, right) in (any::<u64>(), any::<u64>()).prop_filter("distinct entropy", |(left, right)| left != right)
        ) {
            let left_seed = new_seed(32, &left.to_le_bytes());
            let right_seed = new_seed(32, &right.to_le_bytes());
            prop_assert_ne!(left_seed.id, right_seed.id);
        }
    }

    fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
        if let Some(message) = panic.downcast_ref::<String>() {
            return message.clone();
        }
        if let Some(message) = panic.downcast_ref::<&'static str>() {
            return (*message).to_string();
        }
        "<non-string panic>".to_string()
    }
}
