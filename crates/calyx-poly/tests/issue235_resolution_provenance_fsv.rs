//! Issue #235 - resolution provenance integrity.
//!
//! Source of truth: durable AsterVault Anchors/Ledger CF rows plus persisted FSV JSON readback.

#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, anchor_key, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{Anchor, AnchorKind, AnchorValue, CxId, VaultId, VaultStore};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode as decode_ledger};
use calyx_poly::constellation::resolution_anchor;
use calyx_poly::grounding::{
    ERR_GAMMA_DERIVED_AS_UMA, GAMMA_CLOSED_DERIVED_SOURCE_PREFIX, GroundingKind,
    ResolutionSupersession, ResolutionSupersessionKind, grounding_kind_of,
    supersede_gamma_closed_resolution,
};
use calyx_poly::lenses::default_panel;
use calyx_poly::model::{MarketSnapshot, Resolution};
use calyx_poly::pipeline::{ground_market, ingest_snapshot};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const VAULT_SALT: &[u8] = b"poly-issue235-resolution-provenance";

#[test]
fn issue235_resolution_provenance_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE235_FSV_ROOT",
        "poly-issue235-resolution-provenance",
    );
    reset_dir(&root);

    let gamma = gamma_derived_grounding_is_provisional(&root);
    let agreement = uma_supersedes_gamma_on_agreement(&root);
    let correction = uma_corrects_gamma_on_disagreement(&root);
    let legacy = legacy_uma_gamma_source_fails_closed(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 235,
        "proof_claim": "Gamma closed-price-derived outcomes are provenance-tagged separately from UMA finality, carry Provisional trust, and can only be superseded by UMA-final outcomes through an explicit grounding-ledger audit.",
        "minimum_sufficient_proof_corpus": {
            "cases": 4,
            "why_this_is_sufficient": "One Gamma-derived grounding proves the corrected source prefix/trust tier; one UMA agreement proves upgrade audit; one UMA disagreement proves correction audit; one legacy uma:gamma source proves fail-closed classification.",
            "why_larger_is_wasteful": "Additional markets would repeat the same provenance and supersession state transitions without proving new #235 behavior."
        },
        "source_of_truth": "durable AsterVault Anchors/Ledger CF rows plus persisted JSON readback",
        "cases": {
            "gamma_derived": gamma,
            "uma_agreement": agreement,
            "uma_correction": correction,
            "legacy_fail_closed": legacy
        },
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue235_resolution_provenance_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

fn gamma_derived_grounding_is_provisional(root: &Path) -> Value {
    let fixture = ingest_fixture(&root.join("gamma-derived"));
    let gamma = resolution("gamma-closed-derived", true);
    let refs = ground_market(&fixture.vault, &[fixture.cx_id], &gamma, 0).expect("gamma ground");
    fixture.vault.flush().expect("flush gamma case");

    let anchor = read_anchor(&fixture.vault, fixture.cx_id, AnchorKind::TestPass);
    let label = read_anchor(
        &fixture.vault,
        fixture.cx_id,
        AnchorKind::Label("outcome".to_string()),
    );
    let kind = grounding_kind_of(&anchor).expect("classify gamma anchor");
    assert_eq!(kind, GroundingKind::GammaClosedDerived);
    assert_ne!(kind, GroundingKind::ResolvedUma);
    assert_eq!(format!("{:?}", kind.trust()), "Provisional");
    assert_eq!(anchor.source, "gamma-closed-derived:YES");
    assert_eq!(label.source, "gamma-closed-derived:outcome");

    let ledger = ledger_payload(&fixture.vault, refs[0].seq);
    assert_eq!(ledger["event"], json!("poly.outcome_grounding"));
    assert_eq!(ledger["resolution_source"], json!("gamma-closed-derived"));
    persist_case(
        root,
        "gamma-derived",
        json!({
            "anchor_source": anchor.source,
            "label_source": label.source,
            "grounding_kind": format!("{kind:?}"),
            "trust": format!("{:?}", kind.trust()),
            "ledger_seq": refs[0].seq,
            "ledger_payload": ledger
        }),
    )
}

fn uma_supersedes_gamma_on_agreement(root: &Path) -> Value {
    let fixture = ingest_fixture(&root.join("uma-agreement"));
    let gamma = resolution("gamma-closed-derived", true);
    ground_market(&fixture.vault, &[fixture.cx_id], &gamma, 0).expect("gamma ground");
    fixture.vault.flush().expect("flush gamma agreement");
    let gamma_anchor = read_anchor(&fixture.vault, fixture.cx_id, AnchorKind::TestPass);
    let uma_anchor = resolution_anchor(&resolution("uma-onchain", true), 0);
    let supersession =
        supersede_gamma_closed_resolution(&gamma_anchor, &uma_anchor).expect("agreement");
    assert_eq!(
        supersession.kind,
        ResolutionSupersessionKind::UpgradeOnAgreement
    );
    let ledger = append_supersession(&fixture.vault, fixture.cx_id, &supersession);
    persist_case(
        root,
        "uma-agreement",
        json!({
            "gamma_anchor_source": gamma_anchor.source,
            "uma_anchor_source": uma_anchor.source,
            "supersession": supersession,
            "ledger_payload": ledger
        }),
    )
}

fn uma_corrects_gamma_on_disagreement(root: &Path) -> Value {
    let fixture = ingest_fixture(&root.join("uma-correction"));
    let gamma = resolution("gamma-closed-derived", true);
    ground_market(&fixture.vault, &[fixture.cx_id], &gamma, 0).expect("gamma ground");
    fixture.vault.flush().expect("flush gamma correction");
    let gamma_anchor = read_anchor(&fixture.vault, fixture.cx_id, AnchorKind::TestPass);
    let uma_anchor = resolution_anchor(&resolution("uma-onchain", false), 0);
    let supersession =
        supersede_gamma_closed_resolution(&gamma_anchor, &uma_anchor).expect("correction");
    assert_eq!(
        supersession.kind,
        ResolutionSupersessionKind::CorrectionOnDisagreement
    );
    let ledger = append_supersession(&fixture.vault, fixture.cx_id, &supersession);
    persist_case(
        root,
        "uma-correction",
        json!({
            "gamma_anchor_source": gamma_anchor.source,
            "uma_anchor_source": uma_anchor.source,
            "supersession": supersession,
            "ledger_payload": ledger
        }),
    )
}

fn legacy_uma_gamma_source_fails_closed(root: &Path) -> Value {
    let legacy = Anchor {
        kind: AnchorKind::TestPass,
        value: AnchorValue::Bool(true),
        source: "uma:gamma-closed-derived:YES".to_string(),
        observed_at: 1,
        confidence: 1.0,
    };
    let err = grounding_kind_of(&legacy).expect_err("legacy gamma-as-UMA must fail closed");
    assert_eq!(err.code(), ERR_GAMMA_DERIVED_AS_UMA);
    persist_case(
        root,
        "legacy-fail-closed",
        json!({
            "legacy_source": legacy.source,
            "error_code": err.code(),
            "error_message": err.message()
        }),
    )
}

fn append_supersession(
    vault: &AsterVault,
    cx_id: CxId,
    supersession: &ResolutionSupersession,
) -> Value {
    let payload = json!({
        "schema_version": 1,
        "event": "poly.resolution_provenance_supersession",
        "cx_id": cx_id.to_string(),
        "supersession": supersession
    });
    let bytes = serde_json::to_vec(&payload).expect("encode supersession payload");
    let ledger_ref = vault
        .append_ledger_entry(
            EntryKind::Grounding,
            SubjectId::Cx(cx_id),
            bytes,
            ActorId::Service("calyx-poly-grounding".to_string()),
        )
        .expect("append supersession ledger");
    vault.flush().expect("flush supersession ledger");
    let readback = ledger_payload(vault, ledger_ref.seq);
    assert_eq!(readback, payload);
    json!({
        "ledger_seq": ledger_ref.seq,
        "payload": readback
    })
}

fn persist_case(root: &Path, name: &str, value: Value) -> Value {
    let path = root.join(name).join("readback.json");
    write_json(&path, &value);
    let readback: Value =
        serde_json::from_slice(&fs::read(&path).expect("read case")).expect("decode case");
    assert_eq!(readback, value);
    json!({
        "path": path.display().to_string(),
        "readback_equal": true,
        "value": readback
    })
}

fn ingest_fixture(root: &Path) -> Fixture {
    reset_dir(root);
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open vault");
    let panel = default_panel(1, vec!["global".into()]);
    let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let cx_id = ingest_snapshot(&vault, &panel, &snapshot(), vault_id, VAULT_SALT)
        .expect("ingest snapshot");
    Fixture {
        vault_dir,
        vault,
        cx_id,
    }
}

fn read_anchor(vault: &AsterVault, cx_id: CxId, kind: AnchorKind) -> Anchor {
    let row = vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Anchors,
            &anchor_key(cx_id, &kind),
        )
        .expect("read anchor")
        .expect("anchor present");
    encode::decode_anchor(&row).expect("decode anchor")
}

fn ledger_payload(vault: &AsterVault, seq: u64) -> Value {
    let row = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(seq))
        .expect("read ledger")
        .expect("ledger present");
    let ledger = decode_ledger(&row).expect("decode ledger");
    assert_eq!(ledger.kind, EntryKind::Grounding);
    serde_json::from_slice(&ledger.payload).expect("decode payload")
}

fn snapshot() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "yes-token".into(),
        condition_id: "0xissue235".into(),
        outcome_index: 0,
        slug: "issue235-resolution-provenance".into(),
        question: Some("Issue 235 provenance?".into()),
        event_id: None,
        category: Some("crypto".into()),
        region: Some("global".into()),
        tags: Vec::new(),
        resolution_source: Some("gamma-closed-derived".into()),
        neg_risk: false,
        snapshot_ts: 1_785_500_000,
        price: Some(0.62),
        mid: Some(0.62),
        best_bid: Some(0.61),
        best_ask: Some(0.63),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(125_000.0),
        liquidity: Some(40_000.0),
        one_hour_change: None,
        one_day_change: None,
        ofi: None,
        yes_no_residual: None,
        secs_to_resolution: Some(86_400.0),
        holders: Vec::new(),
        makers: Vec::new(),
        counterparty_volumes: Vec::new(),
        onchain_fills: Vec::new(),
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Default::default(),
    }
}

fn resolution(source: &str, our_side_won: bool) -> Resolution {
    Resolution {
        condition_id: "0xissue235".into(),
        winning_outcome_index: if our_side_won { 0 } else { 1 },
        winning_label: if our_side_won { "YES" } else { "NO" }.to_string(),
        resolved_ts: 1_785_600_000,
        source: source.to_string(),
        disputed: false,
    }
}

struct Fixture {
    #[allow(dead_code)]
    vault_dir: PathBuf,
    vault: AsterVault,
    cx_id: CxId,
}

#[allow(dead_code)]
fn _assert_gamma_prefix() {
    assert_eq!(GAMMA_CLOSED_DERIVED_SOURCE_PREFIX, "gamma-closed-derived:");
}
