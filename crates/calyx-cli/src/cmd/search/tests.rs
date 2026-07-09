use std::collections::BTreeMap;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality,
    SlotId, VaultId,
};
use calyx_sextant::{CausalConfidence, FreshnessTag, Hit, PerLensContribution, ProvenanceSource};
use proptest::prelude::*;

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
    assert!(args.resident_addr.is_none());
}

#[test]
fn parse_search_accepts_loopback_resident_addr() {
    let parsed = super::parse_search(&tokens([
        "myvault",
        "hello",
        "--resident-addr",
        "127.0.0.1:8787",
    ]))
    .unwrap();
    let Subcommand::Search(args) = parsed else {
        panic!("expected search subcommand");
    };

    assert_eq!(args.resident_addr, Some("127.0.0.1:8787".parse().unwrap()));
}

#[test]
fn parse_search_rejects_non_loopback_resident_addr() {
    let err =
        super::parse_search(&tokens(["v", "q", "--resident-addr", "10.0.0.10:8787"])).unwrap_err();

    assert_eq!(err.code(), "CALYX_SEARCH_RESIDENT_ADDR_REFUSED");
}

#[test]
fn explain_output_contains_per_lens() {
    let hit = sample_hit(cx(1));
    let rendered = output::render_hits(&[hit], true, true, None);
    let json = serde_json::to_value(rendered).unwrap();

    assert!(json[0]["per_lens"].as_array().is_some());
    assert_eq!(json[0]["per_lens"][0]["slot"], 0);
    assert!(json[0]["provenance"].is_object());
    assert_eq!(json[0]["freshness"]["built_at_seq"], 42);
    assert_eq!(json[0]["freshness"]["base_seq"], 42);
    assert_eq!(json[0]["freshness"]["stale_by"], 0);
    assert_eq!(json[0]["freshness"]["policy"], "fresh_derived");
}

#[test]
fn kernel_answer_ungrounded_error_mentions_remediation() {
    let err = engine::kernel_report_from_docs(&BTreeMap::new(), &[], None).unwrap_err();
    let json = err.to_json();

    assert_eq!(err.code(), "CALYX_KERNEL_UNGROUNDED");
    assert!(json.contains("add anchors"));
}

#[test]
fn kernel_answer_rejects_unmatched_grounded_fallback() {
    let grounded = cx(2);
    let mut docs = BTreeMap::new();
    docs.insert(grounded, anchored_doc(grounded));

    let err = engine::kernel_report_from_docs(&docs, &[sample_hit(cx(9))], None).unwrap_err();

    assert_eq!(err.code(), "CALYX_KERNEL_UNGROUNDED");
    assert!(err.message().contains("no grounded hits"));
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
fn parse_search_rejects_invalid_filter_json_before_vault_open() {
    let err = super::parse_search(&tokens(["v", "q", "--filter", "{not-json"])).unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("parse --filter JSON"));
}

#[test]
fn search_open_options_use_latest_only_router_readback() {
    let options = super::latest_read_vault_options_for_cfs(None);

    assert!(
        !options.restore_mvcc_rows,
        "search/rebuild must not rehydrate every checkpointed durable row into MVCC"
    );
    assert!(
        !options.restore_ledger_hook,
        "search/rebuild must not materialize the full ledger hook for read-only latest-state reads"
    );
    assert!(
        options.read_only,
        "latest search opens must fail closed before any vault mutation"
    );
    assert!(
        options.selected_cfs.is_none(),
        "generic latest read options stay full-CF; callers with narrower needs must pass selected_cfs explicitly"
    );
}

#[test]
fn zero_constellations_render_empty_results() {
    let rendered = output::render_hits(&[], false, true, None);
    assert_eq!(serde_json::to_string(&rendered).unwrap(), "[]");
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

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn anchored_doc(cx_id: CxId) -> Constellation {
    Constellation {
        cx_id,
        vault_id: VaultId::from_ulid(ulid::Ulid::from_bytes([1; 16])),
        panel_version: 1,
        created_at: 1,
        input_ref: InputRef {
            hash: [3; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::TestPass,
            value: AnchorValue::Bool(true),
            source: "unit-test".to_string(),
            observed_at: 1,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq: 1,
            hash: [4; 32],
        },
        flags: CxFlags::default(),
    }
}

fn tokens<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}
