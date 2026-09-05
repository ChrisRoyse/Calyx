//! Issue #1892 — native ordered-block categorical estimator.
//!
//! Hand-computable proof that the native `Multi`/`SegmentedMulti` estimator
//! recovers the categorical association from the ordered block via
//! late-interaction MaxSim landmark features, with no mean/pool of the block's
//! own token or segment vectors. The construction fixes the answer by design:
//!
//! * separated case — the block direction *is* the class, so the block fully
//!   determines the balanced two-class label; the mutual information must
//!   approach the label entropy `H(Y) = 1 bit`.
//! * null case — the block direction is assigned independently of the label, so
//!   the block carries no class information and the mutual information must be
//!   near zero.
//!
//! The estimator feature map is validated with the CPU Ross mixed
//! continuous-discrete KSG estimator (`ksg_mi_continuous_discrete`); the
//! production authority path wraps the identical feature vectors in the
//! strict-CUDA variant.

use std::collections::BTreeMap;

use calyx_assay::ksg_mi_continuous_discrete;
use calyx_core::{
    Asymmetry, CxId, LensId, Modality, QuantPolicy, SegmentVector, SegmentationProvenance, Slot,
    SlotId, SlotKey, SlotShape, SlotState, SlotVector, slot_similarity,
};

use super::representation::assay_block_features;

const DIM: u32 = 16;
const TOKENS: usize = 3;
const PER_CLASS: usize = 96;
const K: usize = 3;

fn block_slot() -> Slot {
    let slot_id = SlotId::new(9);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, "issue1892-block".to_string()),
        lens_id: LensId::from_bytes([9u8; 16]),
        shape: SlotShape::Multi { token_dim: DIM },
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some("unit".to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 1,
    }
}

/// A `Multi` block dominated by unit direction `dir_dim`, with a per-document
/// unique perturbation so no two blocks are identical (the mixed KSG estimator
/// rejects a zero same-class radius from exact duplicates).
fn multi_block(dir_dim: usize, doc_index: usize) -> SlotVector {
    let mut tokens = Vec::with_capacity(TOKENS);
    for token in 0..TOKENS {
        let mut values = vec![0.0f32; DIM as usize];
        values[dir_dim] = 1.0;
        // Unique, small, dominated-by-direction perturbation.
        let bump_dim = 8 + (doc_index % 8);
        values[bump_dim] += 0.02 + 0.0001 * doc_index as f32 + 0.003 * token as f32;
        let spread = 12 + ((doc_index + token) % 4);
        values[spread] += 0.01 * (1 + doc_index % 5) as f32;
        tokens.push(values);
    }
    SlotVector::Multi {
        token_dim: DIM,
        tokens,
    }
}

fn cx(doc_index: usize) -> CxId {
    let mut seed = [0u8; 16];
    seed[0] = (doc_index & 0xff) as u8;
    seed[1] = (doc_index >> 8) as u8;
    seed[2] = 0xA1;
    CxId::from_bytes(seed)
}

/// Builds a balanced two-class sample. When `separated`, block direction equals
/// the class; otherwise direction alternates independently of the class.
fn sample(separated: bool) -> (Vec<(CxId, SlotVector)>, Vec<usize>) {
    let mut owned = Vec::with_capacity(2 * PER_CLASS);
    let mut labels = Vec::with_capacity(2 * PER_CLASS);
    for doc_index in 0..(2 * PER_CLASS) {
        let label = doc_index / PER_CLASS; // 0 for first half, 1 for second half.
        let dir_dim = if separated { label } else { doc_index % 2 };
        owned.push((cx(doc_index), multi_block(dir_dim, doc_index)));
        labels.push(label);
    }
    (owned, labels)
}

fn refs(owned: &[(CxId, SlotVector)]) -> Vec<(CxId, &SlotVector)> {
    owned.iter().map(|(id, vector)| (*id, vector)).collect()
}

#[test]
fn native_block_estimator_recovers_class_association_without_pooling() {
    let slot = block_slot();

    // Separated: the ordered block fully determines the balanced label.
    let (owned, labels) = sample(true);
    let rows = refs(&owned);
    let (features, representation) =
        assay_block_features(&slot, &rows).expect("native block features");

    // No pooling/flattening: the estimator is late-interaction MaxSim, and its
    // feature width is the landmark count (24), never the pooled token_dim (16).
    assert!(
        representation.contains("maxsim_late_interaction"),
        "representation must name the native late-interaction estimator: {representation}"
    );
    assert!(
        !representation.contains("mean") && !representation.contains("pool_token"),
        "representation must not describe a pooled surrogate: {representation}"
    );
    assert_eq!(
        features.len(),
        2 * PER_CLASS,
        "one feature row per document"
    );
    assert_eq!(features[0].len(), 24, "landmark feature width");

    let separated = ksg_mi_continuous_discrete(&features, &labels, K).expect("mixed KSG");
    eprintln!(
        "ISSUE1892 separated bits={} ci=[{},{}] representation={representation}",
        separated.bits, separated.ci_low, separated.ci_high
    );
    assert!(
        f64::from(separated.bits) >= 0.75,
        "clean class separation must approach H(Y)=1 bit; got {} ci=[{},{}]",
        separated.bits,
        separated.ci_low,
        separated.ci_high
    );

    // Null: block direction independent of the label -> no association. A
    // faithful native estimator must NOT manufacture a positive signal here: it
    // either reports near-zero bits or fails closed on low signal.
    let (null_owned, null_labels) = sample(false);
    let null_rows = refs(&null_owned);
    let (null_features, _) =
        assay_block_features(&slot, &null_rows).expect("native block features (null)");
    match ksg_mi_continuous_discrete(&null_features, &null_labels, K) {
        Ok(null) => {
            eprintln!(
                "ISSUE1892 null bits={} ci=[{},{}]",
                null.bits, null.ci_low, null.ci_high
            );
            assert!(
                f64::from(null.bits) <= 0.20,
                "a label-independent block must carry near-zero categorical bits; got {} ci=[{},{}]",
                null.bits,
                null.ci_low,
                null.ci_high
            );
            assert!(
                f64::from(separated.bits) > f64::from(null.bits) + 0.4,
                "separated bits ({}) must dominate null bits ({})",
                separated.bits,
                null.bits
            );
        }
        Err(err) => {
            eprintln!(
                "ISSUE1892 null fail-closed code={} msg={}",
                err.code, err.message
            );
            assert!(
                matches!(
                    err.code,
                    "CALYX_ASSAY_LOW_SIGNAL" | "CALYX_ASSAY_DEGENERATE_INPUT"
                ),
                "null block must fail closed as low-signal, got {}: {}",
                err.code,
                err.message
            );
        }
    }
}

#[test]
fn native_block_estimator_retains_per_segment_blocks_for_segmented_multi() {
    let slot = block_slot();
    // A hand-built, schema-valid two-segment block from the same lens family.
    let provenance = SegmentationProvenance {
        policy_version: 1,
        policy_hash: [7u8; 32],
        source_hash: [9u8; 32],
        source_byte_len: 600,
        source_token_count: 600,
        max_input_tokens: 512,
        special_tokens: 2,
        overlap_tokens: 32,
        max_source_bytes: 100_000,
        max_segments: 64,
    };
    let seg0 = SegmentVector {
        byte_start: 0,
        byte_end: 510,
        token_start: 0,
        token_end: 510,
        measured_tokens: 512,
        data: {
            let mut data = vec![0.0f32; DIM as usize];
            data[0] = 1.0;
            data
        },
    };
    let seg1 = SegmentVector {
        byte_start: 478,
        byte_end: 600,
        token_start: 478,
        token_end: 600,
        measured_tokens: 124,
        data: {
            let mut data = vec![0.0f32; DIM as usize];
            data[1] = 1.0;
            data
        },
    };
    let block = SlotVector::SegmentedMulti {
        token_dim: DIM,
        provenance,
        segments: vec![seg0, seg1],
    };
    block
        .validate_schema()
        .expect("hand-built segmented block is valid");

    // Late-interaction over the block keeps one native block per segment with
    // its source offsets, never a single pooled vector.
    let evidence = slot_similarity(&block, &block).expect("segmented self similarity");
    assert_eq!(evidence.left_block_count, 2, "one block per segment");
    assert!(
        evidence.left_segmentation.is_some(),
        "segmentation provenance is carried natively"
    );
    assert!(evidence.score > 0.999, "self MaxSim ~1.0");
    assert!(
        block.as_dense().is_none(),
        "a segmented block never exposes a pooled dense surrogate"
    );

    // The estimator accepts the real segmented shape and labels it as such.
    let owned = vec![(cx(0), block)];
    let rows = refs(&owned);
    let (features, representation) =
        assay_block_features(&slot, &rows).expect("segmented block features");
    assert_eq!(features.len(), 1);
    assert!(
        representation.starts_with("segmented_multi_maxsim_late_interaction"),
        "segmented shape must be named natively: {representation}"
    );
}

/// End-to-end authority path (#1892): drive the production categorical
/// `calculate()` — which routes typed `Multi` slots through the native
/// ordered-block estimator and the strict-CUDA KSG estimator — over a synthetic
/// panel whose block direction determines the balanced label by construction.
/// The per-slot bits must approach `H(Y)=1 bit` and the reported representation
/// must be the native late-interaction estimator, never a pooled surrogate.
/// Requires the CUDA feature and a GPU (the categorical path is CUDA-strict).
#[cfg(feature = "cuda")]
mod authority {
    use std::collections::BTreeMap;

    use calyx_core::{
        Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, InputRef, LedgerRef, Modality,
        Panel, SlotId, VaultId,
    };

    #[test]
    fn categorical_authority_path_uses_native_block_estimator_over_cuda() {
        let slot = super::block_slot();
        let panel = Panel {
            version: 1,
            slots: vec![slot.clone()],
            created_at: 1,
            kernel_ref: None,
            guard_ref: None,
        };
        let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        let mut docs = BTreeMap::new();
        for doc_index in 0..(2 * super::PER_CLASS) {
            let label = doc_index / super::PER_CLASS;
            let cx_id = super::cx(doc_index);
            let cx = Constellation {
                cx_id,
                vault_id,
                panel_version: 1,
                created_at: doc_index as u64 + 1,
                input_ref: InputRef {
                    hash: [doc_index as u8; 32],
                    pointer: None,
                    redacted: false,
                },
                modality: Modality::Text,
                slots: BTreeMap::from([(SlotId::new(9), super::multi_block(label, doc_index))]),
                scalars: BTreeMap::new(),
                metadata: BTreeMap::new(),
                anchors: vec![Anchor {
                    kind: AnchorKind::Label("topic".to_string()),
                    value: AnchorValue::Enum(format!("class_{label}")),
                    source: "unit".to_string(),
                    observed_at: 1,
                    confidence: 1.0,
                }],
                provenance: LedgerRef {
                    seq: 0,
                    hash: [0u8; 32],
                },
                flags: CxFlags::default(),
            };
            docs.insert(cx_id, cx);
        }

        let out = super::super::calculate(
            &panel,
            &docs,
            &AnchorKind::Label("topic".to_string()),
            "topic",
            false,
            b"issue1892\0topic",
        )
        .expect("categorical authority path");

        let block = out
            .per_slot
            .iter()
            .find(|row| row.slot == 9)
            .expect("block slot measured");
        eprintln!(
            "ISSUE1892_AUTHORITY slot9 bits={} ci={:?} representation={} panel_bits={}",
            block.bits, block.ci, block.representation, out.panel_bits
        );
        assert!(
            block.representation.contains("maxsim_late_interaction"),
            "authority path must use the native estimator: {}",
            block.representation
        );
        assert!(
            !block.representation.contains("mean") && !block.representation.contains("pool_token"),
            "authority path must not pool: {}",
            block.representation
        );
        assert!(
            block.bits >= 0.75,
            "authority-path block bits must approach H(Y)=1 bit; got {} ci={:?}",
            block.bits,
            block.ci
        );
        assert!(out.panel_bits >= 0.75, "panel bits must be high");
    }
}
