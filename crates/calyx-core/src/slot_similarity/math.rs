use std::collections::BTreeMap;

use crate::{Result, SlotVector, SparseEntry};

use super::{
    CALYX_SLOT_SIMILARITY_NUMERICAL, CALYX_SLOT_SIMILARITY_SHAPE, DirectionalBlockMatch,
    SLOT_SIMILARITY_SCHEMA_VERSION, SlotBlockValues, SlotBlockView, SlotSimilarityEvidence,
    block_count_error, similarity_error, slot_ambient_dim, slot_blocks, slot_segmentation,
    slot_similarity_shape,
};

/// Number of independent dot products advanced in lockstep by the dense row
/// kernel. Each accumulator sums its own pair over the shared dimension in the
/// original ascending-index order, so the result is bit-identical to a serial
/// per-pair dot; the only change is that `LANES` independent f64 add chains are
/// in flight at once, hiding FADD latency that a single dependent chain wastes.
/// The token-level `Multi` hot lane (511x511 dense pairs) is where this pays.
const LANES: usize = 4;

/// Compare two typed slots without pooling, concatenating, or densifying them.
pub fn slot_similarity(left: &SlotVector, right: &SlotVector) -> Result<SlotSimilarityEvidence> {
    left.validate_schema()?;
    right.validate_schema()?;
    let left_blocks = slot_blocks(left)?;
    let right_blocks = slot_blocks(right)?;
    let left_dim = slot_ambient_dim(left)?;
    let right_dim = slot_ambient_dim(right)?;
    if left_dim != right_dim {
        return Err(similarity_error(
            CALYX_SLOT_SIMILARITY_SHAPE,
            format!("typed slot dimensions differ: left={left_dim} right={right_dim}"),
            "compare slots with the same ambient dimension or use a typed concat cross-term",
        ));
    }

    let pair_count = u64::try_from(left_blocks.len())
        .ok()
        .and_then(|left| {
            u64::try_from(right_blocks.len())
                .ok()
                .and_then(|right| left.checked_mul(right))
        })
        .ok_or_else(|| {
            similarity_error(
                CALYX_SLOT_SIMILARITY_SHAPE,
                "block-pair count overflow",
                "reduce the bounded block count or repair the stored slot",
            )
        })?;

    // Each block's squared L2 norm is a pure function of that block, so it is
    // computed once here rather than re-summed on every pair (511 norms were
    // recomputed 511x per pair in the token-level Multi hot path). The f64
    // accumulation order is identical to the original inline pass, so the
    // downstream cosines are bit-for-bit unchanged.
    let left_norms: Vec<f64> = left_blocks
        .iter()
        .map(|block| block_sq_norm(&block.values))
        .collect();
    let right_norms: Vec<f64> = right_blocks
        .iter()
        .map(|block| block_sq_norm(&block.values))
        .collect();

    // One left block at a time: fill the whole row of raw dot products (the
    // dense/dense hot path advances `LANES` independent per-pair dots in
    // lockstep), then hash and reduce that row strictly in the original
    // (left_index, right_index) order. Only a single `width`-wide scratch row is
    // held, and no per-improvement `SlotBlockRef` is cloned.
    let width = right_blocks.len();
    let mut dot_row = vec![0.0f64; width];
    let mut left_best: Vec<Option<(usize, f32)>> = vec![None; left_blocks.len()];
    let mut right_best: Vec<Option<(usize, f32)>> = vec![None; width];
    let mut pairwise_hasher = blake3::Hasher::new();
    for (left_index, left_block) in left_blocks.iter().enumerate() {
        fill_dot_row(left_block, &right_blocks, &mut dot_row)?;
        let left_norm = left_norms[left_index];
        for (right_index, right_block) in right_blocks.iter().enumerate() {
            let cosine = finalize_cosine(
                dot_row[right_index],
                left_norm,
                right_norms[right_index],
                left_block,
                right_block,
            )?;
            pairwise_hasher.update(&cosine.to_bits().to_le_bytes());
            retain_best(
                &mut left_best[left_index],
                right_index,
                &right_blocks,
                cosine,
            );
            retain_best(
                &mut right_best[right_index],
                left_index,
                &left_blocks,
                cosine,
            );
        }
    }
    let left_to_right = directional_matches(&left_blocks, &right_blocks, left_best)?;
    let right_to_left = directional_matches(&right_blocks, &left_blocks, right_best)?;
    let left_mean = directional_mean(&left_to_right)?;
    let right_mean = directional_mean(&right_to_left)?;
    let evidence = SlotSimilarityEvidence {
        schema_version: SLOT_SIMILARITY_SCHEMA_VERSION,
        left_shape: slot_similarity_shape(left)?,
        right_shape: slot_similarity_shape(right)?,
        ambient_dim: left_dim,
        left_block_count: u32::try_from(left_blocks.len()).map_err(|_| block_count_error())?,
        right_block_count: u32::try_from(right_blocks.len()).map_err(|_| block_count_error())?,
        evaluated_pair_count: pair_count,
        pairwise_digest: *pairwise_hasher.finalize().as_bytes(),
        left_blocks: left_blocks
            .iter()
            .map(|block| block.reference.clone())
            .collect(),
        right_blocks: right_blocks
            .iter()
            .map(|block| block.reference.clone())
            .collect(),
        left_segmentation: slot_segmentation(left),
        right_segmentation: slot_segmentation(right),
        left_to_right,
        right_to_left,
        score: left_mean.min(right_mean),
    };
    evidence.validate()?;
    Ok(evidence)
}

enum PreparedSlotStorage<'a> {
    One {
        blocks: [SlotBlockView<'a>; 1],
        squared_norms: [f64; 1],
    },
    Many {
        blocks: Vec<SlotBlockView<'a>>,
        squared_norms: Vec<f64>,
    },
}

/// Validated, immutable slot data with block norms prepared for repeated exact
/// comparisons. This retains the native typed blocks and never pools,
/// concatenates, or otherwise substitutes their stored representation.
pub struct PreparedSlotSimilarity<'a> {
    ambient_dim: u32,
    storage: PreparedSlotStorage<'a>,
}

impl PreparedSlotSimilarity<'_> {
    fn blocks(&self) -> &[SlotBlockView<'_>] {
        match &self.storage {
            PreparedSlotStorage::One { blocks, .. } => blocks,
            PreparedSlotStorage::Many { blocks, .. } => blocks,
        }
    }

    fn squared_norms(&self) -> &[f64] {
        match &self.storage {
            PreparedSlotStorage::One { squared_norms, .. } => squared_norms,
            PreparedSlotStorage::Many { squared_norms, .. } => squared_norms,
        }
    }
}

/// Validate one stored slot and prepare its native block norms once for a
/// repeated exact-ranking workload.
pub fn prepare_slot_similarity(vector: &SlotVector) -> Result<PreparedSlotSimilarity<'_>> {
    vector.validate_schema()?;
    let ambient_dim = slot_ambient_dim(vector)?;
    let mut blocks = slot_blocks(vector)?;
    let storage = if blocks.len() == 1 {
        let block = blocks.pop().expect("single prepared slot block");
        let squared_norm = block_sq_norm(&block.values);
        PreparedSlotStorage::One {
            blocks: [block],
            squared_norms: [squared_norm],
        }
    } else {
        let squared_norms = blocks
            .iter()
            .map(|block| block_sq_norm(&block.values))
            .collect();
        PreparedSlotStorage::Many {
            blocks,
            squared_norms,
        }
    };
    Ok(PreparedSlotSimilarity {
        ambient_dim,
        storage,
    })
}

/// Return only the exact fail-closed score for two prepared typed slots.
///
/// The block-pair traversal, f64 accumulation order, cosine clamp, directional
/// maxima, and directional mean are identical to [`slot_similarity`]. The
/// evidence-only digest and coordinate materialization are deliberately omitted
/// because repeated recall ranking consumes only this score.
pub fn prepared_slot_similarity_score(
    left: &PreparedSlotSimilarity<'_>,
    right: &PreparedSlotSimilarity<'_>,
) -> Result<f32> {
    if left.ambient_dim != right.ambient_dim {
        return Err(similarity_error(
            CALYX_SLOT_SIMILARITY_SHAPE,
            format!(
                "typed slot dimensions differ: left={} right={}",
                left.ambient_dim, right.ambient_dim
            ),
            "compare slots with the same ambient dimension or use a typed concat cross-term",
        ));
    }
    let left_blocks = left.blocks();
    let right_blocks = right.blocks();
    let _pair_count = u64::try_from(left_blocks.len())
        .ok()
        .and_then(|left| {
            u64::try_from(right_blocks.len())
                .ok()
                .and_then(|right| left.checked_mul(right))
        })
        .ok_or_else(|| {
            similarity_error(
                CALYX_SLOT_SIMILARITY_SHAPE,
                "block-pair count overflow",
                "reduce the bounded block count or repair the stored slot",
            )
        })?;

    if let ([left_block], [right_block]) = (left_blocks, right_blocks) {
        let dot = block_dot(left_block, right_block)?;
        return finalize_cosine(
            dot,
            left.squared_norms()[0],
            right.squared_norms()[0],
            left_block,
            right_block,
        );
    }

    let mut dot_row = vec![0.0_f64; right_blocks.len()];
    let mut left_best: Vec<Option<(usize, f32)>> = vec![None; left_blocks.len()];
    let mut right_best: Vec<Option<(usize, f32)>> = vec![None; right_blocks.len()];
    for (left_index, left_block) in left_blocks.iter().enumerate() {
        fill_dot_row(left_block, right_blocks, &mut dot_row)?;
        let left_norm = left.squared_norms()[left_index];
        for (right_index, right_block) in right_blocks.iter().enumerate() {
            let cosine = finalize_cosine(
                dot_row[right_index],
                left_norm,
                right.squared_norms()[right_index],
                left_block,
                right_block,
            )?;
            retain_best(
                &mut left_best[left_index],
                right_index,
                right_blocks,
                cosine,
            );
            retain_best(
                &mut right_best[right_index],
                left_index,
                left_blocks,
                cosine,
            );
        }
    }
    let left_mean = directional_best_mean(&left_best)?;
    let right_mean = directional_best_mean(&right_best)?;
    Ok(left_mean.min(right_mean))
}

/// Fills `dots[j]` with the raw dot product of `left` against `right_blocks[j]`,
/// in ascending `j`. When both sides are dense (the token-level `Multi`/segment
/// hot path) the row is computed by [`dense_row_dots`], which advances `LANES`
/// independent accumulators in lockstep; otherwise each pair falls back to the
/// per-pair [`block_dot`]. Every accumulator sums over the shared dimension in
/// ascending index order, identical to the original per-pair loop.
fn fill_dot_row(
    left: &SlotBlockView<'_>,
    right_blocks: &[SlotBlockView<'_>],
    dots: &mut [f64],
) -> Result<()> {
    if let SlotBlockValues::Dense(left_dense) = left.values
        && right_blocks
            .iter()
            .all(|block| matches!(block.values, SlotBlockValues::Dense(_)))
    {
        return dense_row_dots(left_dense, right_blocks, dots);
    }
    for (dot, right) in dots.iter_mut().zip(right_blocks) {
        *dot = block_dot(left, right)?;
    }
    Ok(())
}

/// Dense-against-dense row of dot products, advancing [`LANES`] independent
/// per-pair accumulators in a single dimension sweep. Each `d{i}` accumulates
/// `f64::from(left[k]) * f64::from(right[k])` over ascending `k` — the exact
/// per-pair order — so the row is bit-identical to computing each dot
/// separately; only the FADD-latency-bound single-chain stall is removed.
fn dense_row_dots(
    left: &[f32],
    right_blocks: &[SlotBlockView<'_>],
    dots: &mut [f64],
) -> Result<()> {
    // The unrolled body below reads exactly `LANES` right rows per group.
    const _: () = assert!(LANES == 4, "dense_row_dots body is unrolled for LANES == 4");
    let dim = left.len();
    let width = right_blocks.len();
    let left = &left[..dim];
    let mut j = 0;
    while j + LANES <= width {
        let r0 = dense_row(right_blocks, j, dim)?;
        let r1 = dense_row(right_blocks, j + 1, dim)?;
        let r2 = dense_row(right_blocks, j + 2, dim)?;
        let r3 = dense_row(right_blocks, j + 3, dim)?;
        let (mut d0, mut d1, mut d2, mut d3) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for k in 0..dim {
            let lk = f64::from(left[k]);
            d0 += lk * f64::from(r0[k]);
            d1 += lk * f64::from(r1[k]);
            d2 += lk * f64::from(r2[k]);
            d3 += lk * f64::from(r3[k]);
        }
        dots[j] = d0;
        dots[j + 1] = d1;
        dots[j + 2] = d2;
        dots[j + 3] = d3;
        j += LANES;
    }
    while j < width {
        let r = dense_row(right_blocks, j, dim)?;
        let mut d = 0.0f64;
        for k in 0..dim {
            d += f64::from(left[k]) * f64::from(r[k]);
        }
        dots[j] = d;
        j += 1;
    }
    Ok(())
}

/// Borrows the dense values of `right_blocks[index]`, resliced to `dim` so the
/// hot loop's indexing is provably in-bounds, rejecting a dimension mismatch
/// with the same shape error the original per-pair dense path raised.
fn dense_row<'a>(
    right_blocks: &'a [SlotBlockView<'_>],
    index: usize,
    dim: usize,
) -> Result<&'a [f32]> {
    let SlotBlockValues::Dense(values) = right_blocks[index].values else {
        unreachable!("fill_dot_row guarantees dense right blocks");
    };
    if values.len() != dim || dim == 0 {
        return Err(similarity_error(
            CALYX_SLOT_SIMILARITY_SHAPE,
            format!("dense block dimensions differ: {dim} and {}", values.len()),
            "remeasure under matching frozen slot dimensions",
        ));
    }
    Ok(&values[..dim])
}

/// Combines the (order-preserving) block dot product with the two precomputed
/// squared norms into the final clamped f32 cosine, applying the exact same
/// denominator guard and finiteness checks as the original per-pair path.
fn finalize_cosine(
    dot: f64,
    left_sq_norm: f64,
    right_sq_norm: f64,
    left: &SlotBlockView<'_>,
    right: &SlotBlockView<'_>,
) -> Result<f32> {
    let denominator = left_sq_norm.sqrt() * right_sq_norm.sqrt();
    if !denominator.is_finite() || denominator <= f64::EPSILON {
        return Err(similarity_error(
            CALYX_SLOT_SIMILARITY_NUMERICAL,
            format!(
                "zero/non-finite norm for blocks left={} right={}",
                left.reference.index, right.reference.index
            ),
            "repair or remeasure the invalid slot; zero vectors cannot carry similarity",
        ));
    }
    let cosine = (dot / denominator) as f32;
    if !cosine.is_finite() {
        return Err(similarity_error(
            CALYX_SLOT_SIMILARITY_NUMERICAL,
            "block cosine is non-finite",
            "repair or remeasure the invalid slot",
        ));
    }
    Ok(cosine.clamp(-1.0, 1.0))
}

/// Squared L2 norm of one block, in the same left-to-right f64 order the
/// original inline pass used.
fn block_sq_norm(values: &SlotBlockValues<'_>) -> f64 {
    match values {
        SlotBlockValues::Dense(data) => data
            .iter()
            .map(|value| f64::from(*value) * f64::from(*value))
            .sum(),
        SlotBlockValues::Sparse(entries) => sparse_norm(entries),
    }
}

/// Dot product for one block pair, preserving the exact accumulation order of
/// the original combined-parts routines.
fn block_dot(left: &SlotBlockView<'_>, right: &SlotBlockView<'_>) -> Result<f64> {
    match (&left.values, &right.values) {
        (SlotBlockValues::Dense(left), SlotBlockValues::Dense(right)) => dense_dot(left, right),
        (SlotBlockValues::Sparse(left), SlotBlockValues::Sparse(right)) => {
            Ok(sparse_dot(left, right))
        }
        (SlotBlockValues::Dense(dense), SlotBlockValues::Sparse(sparse)) => {
            dense_sparse_dot(dense, sparse)
        }
        (SlotBlockValues::Sparse(sparse), SlotBlockValues::Dense(dense)) => {
            dense_sparse_dot(dense, sparse)
        }
    }
}

fn dense_dot(left: &[f32], right: &[f32]) -> Result<f64> {
    if left.len() != right.len() || left.is_empty() {
        return Err(similarity_error(
            CALYX_SLOT_SIMILARITY_SHAPE,
            format!(
                "dense block dimensions differ: {} and {}",
                left.len(),
                right.len()
            ),
            "remeasure under matching frozen slot dimensions",
        ));
    }
    Ok(left
        .iter()
        .zip(right)
        .map(|(&left, &right)| f64::from(left) * f64::from(right))
        .sum())
}

fn sparse_dot(left: &[SparseEntry], right: &[SparseEntry]) -> f64 {
    // Sparse schema permits any deterministic entry order. Index only the
    // smaller side so comparison stays sparse and order-independent without
    // allocating an ambient-dimension dense surrogate.
    let (indexed, scanned) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    let indexed = indexed
        .iter()
        .map(|entry| (entry.idx, f64::from(entry.val)))
        .collect::<BTreeMap<_, _>>();
    scanned
        .iter()
        .filter_map(|entry| {
            indexed
                .get(&entry.idx)
                .map(|other| *other * f64::from(entry.val))
        })
        .sum()
}

fn dense_sparse_dot(dense: &[f32], sparse: &[SparseEntry]) -> Result<f64> {
    let mut dot = 0.0;
    for entry in sparse {
        let value = dense.get(entry.idx as usize).ok_or_else(|| {
            similarity_error(
                CALYX_SLOT_SIMILARITY_SHAPE,
                format!(
                    "sparse index {} exceeds dense dim {}",
                    entry.idx,
                    dense.len()
                ),
                "repair the stored sparse slot dimension/index contract",
            )
        })?;
        dot += f64::from(*value) * f64::from(entry.val);
    }
    Ok(dot)
}

fn sparse_norm(entries: &[SparseEntry]) -> f64 {
    entries
        .iter()
        .map(|entry| f64::from(entry.val) * f64::from(entry.val))
        .sum()
}

/// Updates the running best match for one directional row. `candidate` and the
/// stored winner are block positions; the tie-break compares their stored
/// `SlotBlockRef::index` exactly as the original `SlotBlockRef` comparison did,
/// but without cloning a `SlotBlockRef` on every improvement.
fn retain_best(
    best: &mut Option<(usize, f32)>,
    candidate: usize,
    blocks: &[SlotBlockView<'_>],
    cosine: f32,
) {
    let replace = best.as_ref().is_none_or(|(current, current_cosine)| {
        cosine > *current_cosine
            || (cosine == *current_cosine
                && blocks[candidate].reference.index < blocks[*current].reference.index)
    });
    if replace {
        *best = Some((candidate, cosine));
    }
}

fn directional_matches(
    sources: &[SlotBlockView<'_>],
    targets: &[SlotBlockView<'_>],
    matches: Vec<Option<(usize, f32)>>,
) -> Result<Vec<DirectionalBlockMatch>> {
    let mut output = Vec::with_capacity(sources.len());
    for (source, matched) in sources.iter().zip(matches) {
        let (target_index, cosine) = matched.ok_or_else(|| {
            similarity_error(
                CALYX_SLOT_SIMILARITY_SHAPE,
                "directional block has no comparison target",
                "repair the stored slot block count",
            )
        })?;
        output.push(DirectionalBlockMatch {
            source: source.reference.clone(),
            target: targets[target_index].reference.clone(),
            cosine,
        });
    }
    Ok(output)
}

fn directional_mean(matches: &[DirectionalBlockMatch]) -> Result<f32> {
    if matches.is_empty() {
        return Err(similarity_error(
            CALYX_SLOT_SIMILARITY_SHAPE,
            "directional evidence has no blocks",
            "supply a non-empty measured slot",
        ));
    }
    let mean = matches
        .iter()
        .map(|pair| f64::from(pair.cosine))
        .sum::<f64>()
        / matches.len() as f64;
    if !mean.is_finite() {
        return Err(similarity_error(
            CALYX_SLOT_SIMILARITY_NUMERICAL,
            "directional mean similarity is non-finite",
            "repair or remeasure the invalid slot",
        ));
    }
    Ok(mean as f32)
}

fn directional_best_mean(matches: &[Option<(usize, f32)>]) -> Result<f32> {
    if matches.is_empty() {
        return Err(similarity_error(
            CALYX_SLOT_SIMILARITY_SHAPE,
            "directional evidence has no blocks",
            "supply a non-empty measured slot",
        ));
    }
    let mut sum = 0.0_f64;
    for matched in matches {
        let (_, cosine) = matched.ok_or_else(|| {
            similarity_error(
                CALYX_SLOT_SIMILARITY_SHAPE,
                "directional block has no comparison target",
                "repair the stored slot block count",
            )
        })?;
        sum += f64::from(cosine);
    }
    let mean = sum / matches.len() as f64;
    if !mean.is_finite() {
        return Err(similarity_error(
            CALYX_SLOT_SIMILARITY_NUMERICAL,
            "directional mean similarity is non-finite",
            "repair or remeasure the invalid slot",
        ));
    }
    Ok(mean as f32)
}

#[cfg(test)]
mod frozen_evidence_equality {
    //! Proves the optimized [`slot_similarity`] emits byte-identical
    //! [`SlotSimilarityEvidence`] to a verbatim copy of the pre-optimization
    //! reference algorithm across real-shaped random inputs (#1960).

    use super::*;
    use crate::{SegmentVector, SegmentationProvenance};
    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha8Rng;

    // ---- Verbatim pre-#1960 reference implementation (norms recomputed per
    // ---- pair, per-improvement SlotBlockRef clones, inline serial hashing). ----

    fn reference_slot_similarity(
        left: &SlotVector,
        right: &SlotVector,
    ) -> Result<SlotSimilarityEvidence> {
        left.validate_schema()?;
        right.validate_schema()?;
        let left_blocks = slot_blocks(left)?;
        let right_blocks = slot_blocks(right)?;
        let left_dim = slot_ambient_dim(left)?;
        let right_dim = slot_ambient_dim(right)?;
        if left_dim != right_dim {
            return Err(similarity_error(
                CALYX_SLOT_SIMILARITY_SHAPE,
                format!("typed slot dimensions differ: left={left_dim} right={right_dim}"),
                "compare slots with the same ambient dimension or use a typed concat cross-term",
            ));
        }
        let pair_count = u64::try_from(left_blocks.len())
            .ok()
            .and_then(|left| {
                u64::try_from(right_blocks.len())
                    .ok()
                    .and_then(|right| left.checked_mul(right))
            })
            .ok_or_else(|| {
                similarity_error(
                    CALYX_SLOT_SIMILARITY_SHAPE,
                    "block-pair count overflow",
                    "reduce the bounded block count or repair the stored slot",
                )
            })?;
        let mut left_best = vec![None; left_blocks.len()];
        let mut right_best = vec![None; right_blocks.len()];
        let mut pairwise_hasher = blake3::Hasher::new();
        for (left_index, left_block) in left_blocks.iter().enumerate() {
            for (right_index, right_block) in right_blocks.iter().enumerate() {
                let cosine = ref_block_cosine(left_block, right_block)?;
                pairwise_hasher.update(&cosine.to_bits().to_le_bytes());
                ref_retain_best(&mut left_best[left_index], &right_block.reference, cosine);
                ref_retain_best(&mut right_best[right_index], &left_block.reference, cosine);
            }
        }
        let left_to_right = ref_directional_matches(&left_blocks, left_best)?;
        let right_to_left = ref_directional_matches(&right_blocks, right_best)?;
        let left_mean = directional_mean(&left_to_right)?;
        let right_mean = directional_mean(&right_to_left)?;
        let evidence = SlotSimilarityEvidence {
            schema_version: SLOT_SIMILARITY_SCHEMA_VERSION,
            left_shape: slot_similarity_shape(left)?,
            right_shape: slot_similarity_shape(right)?,
            ambient_dim: left_dim,
            left_block_count: u32::try_from(left_blocks.len()).map_err(|_| block_count_error())?,
            right_block_count: u32::try_from(right_blocks.len())
                .map_err(|_| block_count_error())?,
            evaluated_pair_count: pair_count,
            pairwise_digest: *pairwise_hasher.finalize().as_bytes(),
            left_blocks: left_blocks
                .iter()
                .map(|block| block.reference.clone())
                .collect(),
            right_blocks: right_blocks
                .iter()
                .map(|block| block.reference.clone())
                .collect(),
            left_segmentation: slot_segmentation(left),
            right_segmentation: slot_segmentation(right),
            left_to_right,
            right_to_left,
            score: left_mean.min(right_mean),
        };
        evidence.validate()?;
        Ok(evidence)
    }

    fn ref_block_cosine(left: &SlotBlockView<'_>, right: &SlotBlockView<'_>) -> Result<f32> {
        let (dot, left_norm, right_norm) = match (&left.values, &right.values) {
            (SlotBlockValues::Dense(left), SlotBlockValues::Dense(right)) => {
                ref_dense_parts(left, right)?
            }
            (SlotBlockValues::Sparse(left), SlotBlockValues::Sparse(right)) => {
                ref_sparse_parts(left, right)
            }
            (SlotBlockValues::Dense(left), SlotBlockValues::Sparse(right)) => {
                ref_dense_sparse_parts(left, right)?
            }
            (SlotBlockValues::Sparse(left), SlotBlockValues::Dense(right)) => {
                let (dot, right_norm, left_norm) = ref_dense_sparse_parts(right, left)?;
                (dot, left_norm, right_norm)
            }
        };
        let denominator = left_norm.sqrt() * right_norm.sqrt();
        if !denominator.is_finite() || denominator <= f64::EPSILON {
            return Err(similarity_error(
                CALYX_SLOT_SIMILARITY_NUMERICAL,
                "zero norm".to_string(),
                "repair",
            ));
        }
        let cosine = (dot / denominator) as f32;
        if !cosine.is_finite() {
            return Err(similarity_error(
                CALYX_SLOT_SIMILARITY_NUMERICAL,
                "non-finite",
                "repair",
            ));
        }
        Ok(cosine.clamp(-1.0, 1.0))
    }

    fn ref_dense_parts(left: &[f32], right: &[f32]) -> Result<(f64, f64, f64)> {
        if left.len() != right.len() || left.is_empty() {
            return Err(similarity_error(CALYX_SLOT_SIMILARITY_SHAPE, "dim", "fix"));
        }
        let mut dot = 0.0;
        let mut left_norm = 0.0;
        let mut right_norm = 0.0;
        for (&left, &right) in left.iter().zip(right) {
            dot += f64::from(left) * f64::from(right);
            left_norm += f64::from(left) * f64::from(left);
            right_norm += f64::from(right) * f64::from(right);
        }
        Ok((dot, left_norm, right_norm))
    }

    fn ref_sparse_parts(left: &[SparseEntry], right: &[SparseEntry]) -> (f64, f64, f64) {
        let (indexed, scanned) = if left.len() <= right.len() {
            (left, right)
        } else {
            (right, left)
        };
        let indexed = indexed
            .iter()
            .map(|entry| (entry.idx, f64::from(entry.val)))
            .collect::<BTreeMap<_, _>>();
        let dot = scanned
            .iter()
            .filter_map(|entry| {
                indexed
                    .get(&entry.idx)
                    .map(|other| *other * f64::from(entry.val))
            })
            .sum();
        (dot, sparse_norm(left), sparse_norm(right))
    }

    fn ref_dense_sparse_parts(dense: &[f32], sparse: &[SparseEntry]) -> Result<(f64, f64, f64)> {
        let mut dot = 0.0;
        for entry in sparse {
            let value = dense
                .get(entry.idx as usize)
                .ok_or_else(|| similarity_error(CALYX_SLOT_SIMILARITY_SHAPE, "oob", "fix"))?;
            dot += f64::from(*value) * f64::from(entry.val);
        }
        let dense_norm = dense
            .iter()
            .map(|value| f64::from(*value) * f64::from(*value))
            .sum();
        Ok((dot, dense_norm, sparse_norm(sparse)))
    }

    fn ref_retain_best(
        best: &mut Option<(super::super::SlotBlockRef, f32)>,
        target: &super::super::SlotBlockRef,
        cosine: f32,
    ) {
        let replace = best
            .as_ref()
            .is_none_or(|(current_target, current_cosine)| {
                cosine > *current_cosine
                    || (cosine == *current_cosine && target.index < current_target.index)
            });
        if replace {
            *best = Some((target.clone(), cosine));
        }
    }

    fn ref_directional_matches(
        sources: &[SlotBlockView<'_>],
        matches: Vec<Option<(super::super::SlotBlockRef, f32)>>,
    ) -> Result<Vec<DirectionalBlockMatch>> {
        let mut output = Vec::with_capacity(sources.len());
        for (source, matched) in sources.iter().zip(matches) {
            let (target, cosine) = matched
                .ok_or_else(|| similarity_error(CALYX_SLOT_SIMILARITY_SHAPE, "no target", "fix"))?;
            output.push(DirectionalBlockMatch {
                source: source.reference.clone(),
                target,
                cosine,
            });
        }
        Ok(output)
    }

    // ---- Real-shaped random generators ----

    fn dense(rng: &mut ChaCha8Rng, dim: u32) -> SlotVector {
        SlotVector::Dense {
            dim,
            data: (0..dim).map(|_| rng.random_range(-1.0..1.0)).collect(),
        }
    }

    fn sparse(rng: &mut ChaCha8Rng, dim: u32, nnz: u32) -> SlotVector {
        // Strictly increasing unique indices with non-zero values.
        let mut entries = Vec::new();
        let mut idx = 0u32;
        for _ in 0..nnz {
            idx += rng.random_range(1..=3);
            if idx >= dim {
                break;
            }
            entries.push(SparseEntry {
                idx,
                val: rng.random_range(0.1..1.0),
            });
        }
        SlotVector::Sparse { dim, entries }
    }

    fn multi(rng: &mut ChaCha8Rng, token_dim: u32, tokens: usize) -> SlotVector {
        SlotVector::Multi {
            token_dim,
            tokens: (0..tokens)
                .map(|_| {
                    (0..token_dim)
                        .map(|_| rng.random_range(-1.0..1.0))
                        .collect()
                })
                .collect(),
        }
    }

    fn segmented(rng: &mut ChaCha8Rng, token_dim: u32, n: u32) -> SlotVector {
        // overlap=0 contiguous segments, 4 content tokens + 2 special per segment.
        const T: u32 = 4;
        const BYTES: u64 = 10;
        const SPECIAL: u16 = 2;
        let provenance = SegmentationProvenance {
            policy_version: 1,
            policy_hash: [0u8; 32],
            source_hash: [0u8; 32],
            source_byte_len: BYTES * u64::from(n),
            source_token_count: T * n,
            max_input_tokens: T + u32::from(SPECIAL),
            special_tokens: SPECIAL,
            overlap_tokens: 0,
            max_source_bytes: BYTES * u64::from(n),
            max_segments: n,
        };
        let segments = (0..n)
            .map(|i| SegmentVector {
                byte_start: BYTES * u64::from(i),
                byte_end: BYTES * u64::from(i + 1),
                token_start: T * i,
                token_end: T * (i + 1),
                measured_tokens: T + u32::from(SPECIAL),
                data: (0..token_dim)
                    .map(|_| rng.random_range(-1.0..1.0))
                    .collect(),
            })
            .collect();
        SlotVector::SegmentedMulti {
            token_dim,
            provenance,
            segments,
        }
    }

    fn assert_identical(left: &SlotVector, right: &SlotVector) {
        let reference = reference_slot_similarity(left, right).expect("reference evidence");
        let optimized = slot_similarity(left, right).expect("optimized evidence");
        assert_eq!(
            reference, optimized,
            "optimized evidence diverged from the frozen reference"
        );
        // Full-struct PartialEq already covers the digest, but assert it
        // explicitly so a regression names the frozen field.
        assert_eq!(
            reference.pairwise_digest, optimized.pairwise_digest,
            "pairwise_digest is frozen search/kernel evidence"
        );
    }

    #[test]
    fn evidence_is_bit_identical_across_real_shapes() {
        let mut rng = ChaCha8Rng::seed_from_u64(0x1960);
        for _ in 0..24 {
            // Dense
            assert_identical(&dense(&mut rng, 1024), &dense(&mut rng, 1024));
            // Sparse
            assert_identical(&sparse(&mut rng, 30000, 120), &sparse(&mut rng, 30000, 90));
            // SegmentedMulti 768 x 2..=7: widths below LANES exercise the
            // dense-row tail loop (0 full groups).
            let ln = rng.random_range(2..=7);
            let rn = rng.random_range(2..=7);
            assert_identical(&segmented(&mut rng, 768, ln), &segmented(&mut rng, 768, rn));
            // Multi 1024 x 8: exactly two full LANES groups, no tail.
            assert_identical(&multi(&mut rng, 1024, 8), &multi(&mut rng, 1024, 8));
            // Multi 1024 x {48,40}: many full groups; 40 % LANES == 0.
            assert_identical(&multi(&mut rng, 1024, 48), &multi(&mut rng, 1024, 40));
        }
    }

    #[test]
    fn evidence_is_bit_identical_for_hot_shape() {
        // The exact production hot lane: token-level Multi 1024 x 511.
        let mut rng = ChaCha8Rng::seed_from_u64(0x511);
        let left = multi(&mut rng, 1024, 511);
        let right = multi(&mut rng, 1024, 511);
        assert_identical(&left, &right);
    }

    /// Single-threaded wall-clock comparison of the pre-#1960 reference vs the
    /// optimized (LANES-wide ILP) path on the production hot lane (token-level
    /// Multi 1024 x 511 = 261,121 dense block pairs). This isolates the per-core
    /// throughput win, which composes with the caller's existing candidate-level
    /// parallelism. Ignored by default; run with:
    ///   cargo test -p calyx-core --release -- --ignored --nocapture bench_hot_shape
    #[test]
    #[ignore = "perf benchmark; run explicitly in --release"]
    fn bench_hot_shape() {
        let mut rng = ChaCha8Rng::seed_from_u64(0xB0);
        let left = multi(&mut rng, 1024, 511);
        let right = multi(&mut rng, 1024, 511);
        // Warm up + verify identity once.
        assert_identical(&left, &right);

        let iters = 20;
        let start = std::time::Instant::now();
        for _ in 0..iters {
            let _ = reference_slot_similarity(&left, &right).expect("reference");
        }
        let reference = start.elapsed();

        let start = std::time::Instant::now();
        for _ in 0..iters {
            let _ = slot_similarity(&left, &right).expect("optimized");
        }
        let optimized = start.elapsed();

        eprintln!(
            "bench_hot_shape (Multi 1024x511, {iters} iters): reference={:?} ({:.2} ms/call), optimized={:?} ({:.2} ms/call), speedup={:.2}x",
            reference,
            reference.as_secs_f64() * 1e3 / iters as f64,
            optimized,
            optimized.as_secs_f64() * 1e3 / iters as f64,
            reference.as_secs_f64() / optimized.as_secs_f64(),
        );
    }
}
