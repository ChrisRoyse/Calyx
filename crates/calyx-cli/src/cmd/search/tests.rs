use std::collections::BTreeMap;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality,
    SlotId, SlotVector, VaultId,
};
use calyx_sextant::{
    CausalConfidence, FreshnessTag, FusionStrategy, Hit, PerLensContribution, ProvenanceSource,
};
use proptest::prelude::*;
use ulid::Ulid;

use super::super::Subcommand;
use super::engine;
use super::output;
use super::parse::{SearchFusionArg, SearchGuardArg};

#[test]
fn parse_search_defaults_to_rrf_guard_off_and_provenance() {
    let parsed = super::parse_search(&tokens(["myvault", "hello", "--k", "5"])).unwrap();
    let Subcommand::Search(args) = parsed else {
        panic!("expected search subcommand");
    };

    assert_eq!(args.k, 5);
    assert_eq!(args.fusion, SearchFusionArg::Rrf);
    assert_eq!(args.guard, SearchGuardArg::Off);
    assert!(!args.explain);
    assert!(args.provenance);

    let query = args.to_query(&[SlotId::new(0)]).unwrap();
    assert_eq!(query.k, 5);
    assert_eq!(query.fusion, Some(FusionStrategy::Rrf));
    assert!(!query.explain);
    assert!(query.require_stored_provenance);
}

#[test]
fn explain_output_contains_per_lens() {
    let hit = sample_hit(cx(1));
    let rendered = output::render_hits(&[hit], true, true, None);
    let json = serde_json::to_value(rendered).unwrap();

    assert!(json[0]["per_lens"].as_array().is_some());
    assert_eq!(json[0]["per_lens"][0]["slot"], 0);
    assert!(json[0]["provenance"].is_object());
}

#[test]
fn kernel_answer_ungrounded_error_mentions_remediation() {
    let err = engine::kernel_report_from_docs(&BTreeMap::new(), &[], None).unwrap_err();
    let json = err.to_json();

    assert_eq!(err.code(), "CALYX_KERNEL_UNGROUNDED");
    assert!(json.contains("add anchors"));
}

#[test]
fn k_zero_and_unknown_fusion_are_usage_errors() {
    let k_err = super::parse_search(&tokens(["v", "q", "--k", "0"])).unwrap_err();
    assert_eq!(k_err.code(), "CALYX_CLI_USAGE_ERROR");

    let fusion_err =
        super::parse_search(&tokens(["v", "q", "--fusion", "unknown-mode"])).unwrap_err();
    assert_eq!(fusion_err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn zero_constellations_render_empty_results() {
    let rendered = output::render_hits(&[], false, true, None);
    assert_eq!(serde_json::to_string(&rendered).unwrap(), "[]");
}

#[test]
fn non_empty_search_without_indexable_vectors_fails_closed() {
    assert_eq!(
        engine::no_indexable_query_vectors().code,
        "CALYX_STALE_DERIVED"
    );
    assert_eq!(
        engine::no_indexable_stored_vectors().code,
        "CALYX_STALE_DERIVED"
    );
}

#[test]
fn guard_rejects_orthogonal_dense_hit() {
    let slot = SlotId::new(0);
    let id = cx(2);
    let mut docs = BTreeMap::new();
    docs.insert(id, constellation(id, vec![0.0, 1.0], Vec::new()));
    let hit = sample_hit(id);
    let query_vectors = vec![(
        slot,
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0, 0.0],
        },
    )];

    assert!(!engine::guard_keeps_hit_for_test(
        &hit,
        &docs,
        &query_vectors
    ));
}

#[test]
fn parse_kernel_answer_accepts_anchor_and_explain() {
    let parsed = super::parse_kernel_answer(&tokens([
        "myvault",
        "hello",
        "--anchor",
        "label:gold",
        "--explain",
    ]))
    .unwrap();
    let Subcommand::KernelAnswer(args) = parsed else {
        panic!("expected kernel-answer subcommand");
    };

    assert_eq!(args.anchor.as_deref(), Some("label:gold"));
    assert!(args.explain);
}

proptest! {
    #[test]
    fn hit_output_preserves_cx_hex(bytes in any::<[u8; 16]>()) {
        let id = CxId::from_bytes(bytes);
        let rendered = output::render_hits(&[sample_hit(id)], false, true, None);
        let json = serde_json::to_value(rendered).unwrap();
        let encoded = json[0]["cx_id"].as_str().unwrap();
        let decoded = encoded.parse::<CxId>().unwrap();

        prop_assert_eq!(decoded.as_bytes(), id.as_bytes());
    }
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
            rank: 2,
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

fn constellation(cx_id: CxId, dense: Vec<f32>, anchors: Vec<Anchor>) -> Constellation {
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
        anchors,
        provenance: LedgerRef {
            seq: 1,
            hash: [1; 32],
        },
        flags: CxFlags::default(),
    }
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

#[allow(dead_code)]
fn anchor(kind: AnchorKind) -> Anchor {
    Anchor {
        kind,
        value: AnchorValue::Bool(true),
        source: "unit".to_string(),
        observed_at: 1,
        confidence: 1.0,
    }
}

fn tokens<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}
