use super::*;
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
};
use calyx_sextant::{CausalConfidence, FreshnessTag, Hit, PerLensContribution, ProvenanceSource};
use std::collections::BTreeMap;
use ulid::Ulid;

#[test]
fn no_indexable_errors_are_stale_derived() {
    assert_eq!(no_indexable_query_vectors().code, "CALYX_STALE_DERIVED");
    assert_eq!(no_indexable_stored_vectors().code, "CALYX_STALE_DERIVED");
}

#[test]
fn explicit_fusion_choices_preserve_profile_and_slot() {
    let slots = [SlotId::new(8), SlotId::new(14)];
    assert_eq!(
        FusionChoice::WeightedRrfProfile(RrfProfile::Bridge)
            .to_strategy(&slots)
            .unwrap(),
        FusionStrategy::WeightedRrf {
            profile: RrfProfile::Bridge
        }
    );
    assert_eq!(
        FusionChoice::SingleLensSlot(SlotId::new(14))
            .to_strategy(&slots)
            .unwrap(),
        FusionStrategy::SingleLens {
            slot: SlotId::new(14)
        }
    );
    let err = FusionChoice::SingleLensSlot(SlotId::new(99))
        .to_strategy(&slots)
        .unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn cosine_is_one_for_identical_and_zero_for_orthogonal() {
    assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0]), Some(1.0));
    assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), Some(0.0));
    assert_eq!(cosine(&[1.0], &[1.0, 0.0]), None);
    assert_eq!(cosine(&[], &[]), None);
}

#[test]
fn in_region_guard_rejects_orthogonal_dense_hit() {
    let slot = SlotId::new(0);
    let id = cx(2);
    let mut docs = BTreeMap::new();
    docs.insert(id, constellation(id, vec![0.0, 1.0]));
    let hit = sample_hit(id);
    let query_vectors = vec![(
        slot,
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0, 0.0],
        },
    )];

    // Orthogonal query vs stored vector -> cosine 0 < GUARD_TAU -> filtered out.
    let kept = apply_in_region_guard(vec![hit], &docs, &query_vectors);
    assert!(kept.is_empty(), "orthogonal hit must be guard-rejected");
}

#[test]
fn in_region_guard_keeps_aligned_dense_hit() {
    let slot = SlotId::new(0);
    let id = cx(3);
    let mut docs = BTreeMap::new();
    docs.insert(id, constellation(id, vec![1.0, 0.0]));
    let hit = sample_hit(id);
    let query_vectors = vec![(
        slot,
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0, 0.0],
        },
    )];
    let kept = apply_in_region_guard(vec![hit], &docs, &query_vectors);
    assert_eq!(
        kept.len(),
        1,
        "identical vector (cosine 1.0) must pass the guard"
    );
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn sample_hit(cx_id: CxId) -> Hit {
    Hit {
        cx_id,
        score: 0.834,
        rank: 1,
        event_time_secs: None,
        temporal_scores: None,
        causal_confidence: CausalConfidence::Absent,
        causal_gate: None,
        per_lens: vec![PerLensContribution {
            slot: SlotId::new(0),
            rank: 1,
            raw_score: 0.91,
            weight: 0.5,
            contribution: 0.455,
        }],
        cross_terms_used: false,
        guard: None,
        provenance: LedgerRef {
            seq: 42,
            hash: [7; 32],
        },
        provenance_source: ProvenanceSource::Stored,
        freshness: FreshnessTag::fresh(42),
        explain: None,
    }
}

fn constellation(cx_id: CxId, dense: Vec<f32>) -> Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: dense.len() as u32,
            data: dense,
        },
    );
    Constellation {
        cx_id,
        vault_id: VaultId::from_ulid(Ulid::from_bytes([9; 16])),
        panel_version: 1,
        created_at: 1,
        input_ref: InputRef {
            hash: [0; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [1; 32],
        },
        flags: CxFlags::default(),
    }
}
