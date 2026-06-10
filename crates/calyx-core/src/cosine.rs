//! Shared dense cosine helpers.

use std::collections::BTreeMap;

use crate::SlotId;

/// Per-slot tau lookup used by guard-like policies without coupling crates.
pub trait GuardTauProfile {
    fn tau_for(&self, slot: &SlotId) -> Option<f32>;
}

impl GuardTauProfile for BTreeMap<SlotId, f32> {
    fn tau_for(&self, slot: &SlotId) -> Option<f32> {
        self.get(slot).copied()
    }
}

/// Computes cosine for two dense vectors, failing closed on invalid vectors.
pub fn dense_cosine(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (left, right) in left.iter().zip(right) {
        if !left.is_finite() || !right.is_finite() {
            return None;
        }
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    let denom = left_norm.sqrt() * right_norm.sqrt();
    if !denom.is_finite() || denom <= 0.0 {
        return None;
    }
    let cosine = dot / denom;
    cosine.is_finite().then_some(cosine)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_cosine_matches_known_unit_vectors() {
        assert_eq!(dense_cosine(&[1.0, 0.0], &[1.0, 0.0]), Some(1.0));
        assert_eq!(dense_cosine(&[1.0, 0.0], &[0.0, 1.0]), Some(0.0));
    }

    #[test]
    fn dense_cosine_fails_invalid_vectors_closed() {
        assert_eq!(dense_cosine(&[], &[]), None);
        assert_eq!(dense_cosine(&[1.0], &[1.0, 0.0]), None);
        assert_eq!(dense_cosine(&[0.0, 0.0], &[1.0, 0.0]), None);
        assert_eq!(dense_cosine(&[f32::NAN], &[1.0]), None);
    }
}
