use calyx_core::{CalyxError, CxId, Slot, SlotVector, slot_similarity};

use super::contextual_assay_error;
use crate::error::{CliError, CliResult};

const SPARSE_HASH_DIM: usize = 2_048;
/// Deterministic, label-blind landmark budget for the native ordered-block
/// (Multi/SegmentedMulti) categorical estimator. Each block is represented by
/// its late-interaction MaxSim profile against these landmark blocks — never by
/// a pooled/mean surrogate of its own token or segment vectors.
const BLOCK_LANDMARK_TARGET: usize = 24;

pub(super) fn assay_vector(
    slot: &Slot,
    cx_id: CxId,
    stored: &SlotVector,
) -> CliResult<(Vec<f32>, String)> {
    stored.validate_schema().map_err(|error| {
        contextual_assay_error(
            error,
            format!(
                "stored vector schema failed for active slot {} ({}) on cx {cx_id}",
                slot.slot_id,
                slot.slot_key.key()
            ),
        )
    })?;
    let (mut vector, representation) = match stored {
        SlotVector::Dense { dim, data } => (data.clone(), format!("dense_native_dim_{dim}_l2")),
        SlotVector::Sparse { dim, entries } => {
            let mut projected = vec![0.0f32; SPARSE_HASH_DIM];
            for entry in entries {
                let (bucket, sign) = sparse_hash(slot.slot_id.get(), entry.idx);
                projected[bucket] += sign * entry.val;
            }
            (
                projected,
                format!("sparse_signed_blake3_hash_dim_{dim}_to_{SPARSE_HASH_DIM}_l2"),
            )
        }
        SlotVector::Multi { token_dim, .. } => {
            return Err(CalyxError::assay_degenerate_input(format!(
                "categorical bits refuses typed Multi{{token_dim:{token_dim}}} for active slot {} ({}) on cx {cx_id}; pooling token vectors would change late-interaction meaning, so use a native ordered-block estimator",
                slot.slot_id,
                slot.slot_key.key(),
            ))
            .into());
        }
        SlotVector::SegmentedMulti {
            token_dim,
            segments,
            ..
        } => {
            return Err(CalyxError::assay_degenerate_input(format!(
                "categorical bits refuses typed SegmentedMulti{{token_dim:{token_dim},segments:{}}} for active slot {} ({}) on cx {cx_id}; pooling document segments would erase source-bound meaning, so use a native ordered-block estimator",
                segments.len(),
                slot.slot_id,
                slot.slot_key.key(),
            ))
            .into());
        }
        SlotVector::Absent { reason } => {
            return Err(CalyxError::assay_degenerate_input(format!(
                "active slot {} ({}) is explicitly absent on cx {cx_id}: {reason:?}",
                slot.slot_id,
                slot.slot_key.key()
            ))
            .into());
        }
    };
    normalize_nonzero(&mut vector, slot, cx_id)?;
    Ok((vector, representation))
}

/// Native ordered-block categorical estimator (#1892).
///
/// Represents every `Multi`/`SegmentedMulti` slot in the sample by its
/// late-interaction MaxSim profile against a deterministic, label-blind set of
/// landmark blocks drawn from the same sample. Each coordinate is a genuine
/// block-to-block MaxSim computed by [`slot_similarity`], which retains every
/// token/segment block and its source offsets — no token or segment vector is
/// ever mean/max pooled or concatenated into a substitute vector. The resulting
/// fixed-width, L2-normalized coordinates feed the same KSG continuous-discrete
/// estimator the dense lanes use, so the ordered block participates in the joint
/// panel natively rather than being collapsed or refused.
pub(super) fn assay_block_features(
    slot: &Slot,
    rows: &[(CxId, &SlotVector)],
) -> CliResult<(Vec<Vec<f32>>, String)> {
    let (shape, token_dim) = block_shape(slot, rows)?;
    let sample_len = rows.len();
    let landmark_count = BLOCK_LANDMARK_TARGET.min(sample_len);
    if landmark_count == 0 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "native ordered-block estimator for active slot {} ({}) has no sampled blocks",
            slot.slot_id,
            slot.slot_key.key()
        ))
        .into());
    }
    // Deterministic, label-blind landmark stride over the cx_id-ordered sample.
    let landmarks = (0..landmark_count)
        .map(|index| index * sample_len / landmark_count)
        .collect::<Vec<_>>();
    let mut features = Vec::with_capacity(sample_len);
    for &(cx_id, stored) in rows {
        let mut profile = Vec::with_capacity(landmark_count);
        for &landmark in &landmarks {
            let evidence = slot_similarity(stored, rows[landmark].1).map_err(|error| {
                contextual_assay_error(
                    error,
                    format!(
                        "native ordered-block MaxSim failed for active slot {} ({}) on cx {cx_id} against landmark row {landmark}",
                        slot.slot_id,
                        slot.slot_key.key()
                    ),
                )
            })?;
            profile.push(evidence.score);
        }
        normalize_nonzero(&mut profile, slot, cx_id)?;
        features.push(profile);
    }
    let representation = format!(
        "{shape}_maxsim_late_interaction_landmark_{landmark_count}_tokendim_{token_dim}_l2"
    );
    Ok((features, representation))
}

/// Confirms the whole sample carries one native block shape at one token
/// dimension. A mixed or non-block sample is a degenerate categorical input.
fn block_shape(slot: &Slot, rows: &[(CxId, &SlotVector)]) -> CliResult<(&'static str, u32)> {
    let first = rows.first().ok_or_else(|| {
        CliError::from(CalyxError::assay_insufficient_samples(format!(
            "native ordered-block estimator for active slot {} ({}) has no sampled rows",
            slot.slot_id,
            slot.slot_key.key()
        )))
    })?;
    let (shape, token_dim) = match first.1 {
        SlotVector::Multi { token_dim, .. } => ("multi", *token_dim),
        SlotVector::SegmentedMulti { token_dim, .. } => ("segmented_multi", *token_dim),
        other => {
            return Err(CalyxError::assay_degenerate_input(format!(
                "native ordered-block estimator for active slot {} ({}) requires Multi/SegmentedMulti blocks; cx {} stores {other:?}",
                slot.slot_id,
                slot.slot_key.key(),
                first.0
            ))
            .into());
        }
    };
    for &(cx_id, stored) in rows {
        let matches = match (shape, stored) {
            ("multi", SlotVector::Multi { token_dim: dim, .. })
            | ("segmented_multi", SlotVector::SegmentedMulti { token_dim: dim, .. }) => {
                *dim == token_dim
            }
            _ => false,
        };
        if !matches {
            return Err(CalyxError::assay_degenerate_input(format!(
                "native ordered-block estimator for active slot {} ({}) requires one {shape}{{token_dim:{token_dim}}} shape across the sample; cx {cx_id} differs",
                slot.slot_id,
                slot.slot_key.key()
            ))
            .into());
        }
    }
    Ok((shape, token_dim))
}

fn sparse_hash(slot: u16, index: u32) -> (usize, f32) {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"calyx-assay-sparse-feature-hash-v1");
    hasher.update(&slot.to_le_bytes());
    hasher.update(&index.to_le_bytes());
    let digest = hasher.finalize();
    let bytes = digest.as_bytes();
    let bucket = u64::from_le_bytes(bytes[0..8].try_into().expect("eight hash bytes")) as usize
        % SPARSE_HASH_DIM;
    let sign = if bytes[8] & 1 == 0 { 1.0 } else { -1.0 };
    (bucket, sign)
}

fn normalize_nonzero(vector: &mut [f32], slot: &Slot, cx_id: CxId) -> CliResult {
    let norm_sq = vector
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>();
    if !norm_sq.is_finite() || norm_sq <= 0.0 {
        return Err(CalyxError::assay_degenerate_input(format!(
            "active slot {} ({}) produces a non-finite or zero-norm assay vector on cx {cx_id}: norm_sq={norm_sq}",
            slot.slot_id,
            slot.slot_key.key()
        ))
        .into());
    }
    let norm = norm_sq.sqrt() as f32;
    for value in vector {
        *value /= norm;
    }
    Ok(())
}

pub(super) fn cosine_to_centroid(vectors: &[Vec<f32>], slot: &Slot) -> CliResult<Vec<f32>> {
    let dim = vectors.first().map_or(0, Vec::len);
    let mut centroid = vec![0.0f64; dim];
    for vector in vectors {
        for (sum, value) in centroid.iter_mut().zip(vector) {
            *sum += f64::from(*value);
        }
    }
    let centroid_norm = centroid
        .iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    if !centroid_norm.is_finite() || centroid_norm <= 0.0 {
        return Err(CalyxError::assay_degenerate_input(format!(
            "pairwise redundancy centroid is zero or non-finite for slot {} ({})",
            slot.slot_id,
            slot.slot_key.key()
        ))
        .into());
    }
    let mut scores = Vec::with_capacity(vectors.len());
    for (row_index, vector) in vectors.iter().enumerate() {
        let dot = vector
            .iter()
            .zip(&centroid)
            .map(|(left, right)| f64::from(*left) * *right)
            .sum::<f64>();
        let vector_norm = vector
            .iter()
            .map(|value| f64::from(*value) * f64::from(*value))
            .sum::<f64>()
            .sqrt();
        let score = dot / (vector_norm * centroid_norm);
        if !score.is_finite() {
            return Err(CalyxError::assay_degenerate_input(format!(
                "pairwise redundancy cosine is non-finite for slot {} ({}) at sample row {row_index}",
                slot.slot_id,
                slot.slot_key.key()
            ))
            .into());
        }
        scores.push(score as f32);
    }
    Ok(scores)
}
