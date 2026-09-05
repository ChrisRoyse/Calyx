//! Issue #209 — Provisional→Trusted trust lifecycle on proxy anchors, Full State Verification.
//!
//! Source of truth: the on-disk `panel_diagnostics_*.json` records (their `TrustTag`), the anchor
//! confidences, and the persisted trust-transition entry — each read back separately. The lifecycle:
//! a bit/diagnostic measured against a **proxy** anchor is Provisional and a provisional-only
//! forecast is refused; when the market resolves, a backfill promotes it to Trusted and the forecast
//! becomes admissible. No silent promotion; every fail-closed edge is exercised.

use std::path::Path;

use calyx_assay::{TotalCorrelationConfig, TrustTag};
use calyx_core::{Anchor, AnchorKind, AnchorValue, FixedClock};
use calyx_poly::admission::{
    AdmissionInputs, AdmissionParams, REFUSE_PROVISIONAL_ONLY, admit_forecast,
    refuse_if_provisional_only,
};
use calyx_poly::grounding::{
    ERR_BACKFILL_CONTRADICTION, ERR_PROXY_CONFIDENCE, ProxyKind, grounding_kind_of,
    promote_on_resolution, proxy_anchor,
};
use calyx_poly::panel_diagnostics::{
    PanelDiagnosticsConfig, PanelMatrix, compute_panel_diagnostics, read_panel_diagnostics,
    write_panel_diagnostics,
};
use calyx_poly::risk::MarketIntegrityScreen;
use serde_json::json;

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
// calyx-shared-module: path=synthetic_panels.rs alias=__calyx_shared_synthetic_panels_rs local=synthetic visibility=private
use crate::__calyx_shared_synthetic_panels_rs as synthetic;

use support::{
    known_healthy_market_integrity, known_healthy_oracle_risk, known_healthy_wash_trade,
    named_fsv_root, reset_dir, write_blake3sums, write_json,
};
use synthetic::independent;

const PANEL_VERSION: u32 = 1;

fn cfg() -> PanelDiagnosticsConfig {
    PanelDiagnosticsConfig {
        tc: TotalCorrelationConfig {
            k: 3,
            bootstrap_resamples: 60,
            ..TotalCorrelationConfig::default()
        },
    }
}

fn resolved_anchor(won: bool, i: usize) -> Anchor {
    Anchor {
        kind: AnchorKind::TestPass,
        value: AnchorValue::Bool(won),
        source: "uma:polymarket".to_string(),
        observed_at: i as u64,
        confidence: 1.0,
    }
}

#[test]
fn issue209_trust_lifecycle_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE209_FSV_ROOT", "poly-issue209-trust-lifecycle");
    reset_dir(&root);
    let clock = FixedClock::new(1_785_400_000);

    edge_proxy_confidence_one_rejected(&root);
    happy_proxy_then_resolved_lifecycle(&root, &clock);
    edge_resolution_contradicts_proxy(&root);
    edge_backfill_unresolved_no_promotion(&root);

    write_blake3sums(&root);
}

/// Edge: a proxy anchor with confidence 1.0 is rejected at construction — a proxy is an estimate.
fn edge_proxy_confidence_one_rejected(root: &Path) {
    let err =
        proxy_anchor(ProxyKind::Up24h, true, 1.0, 1_785_000_000).expect_err("must reject 1.0");
    assert_eq!(err.code(), ERR_PROXY_CONFIDENCE);
    // A valid proxy carries confidence strictly inside (0, 1).
    let ok = proxy_anchor(ProxyKind::Up24h, true, 0.62, 1_785_000_000).expect("valid proxy");
    assert!(ok.confidence > 0.0 && ok.confidence < 1.0);
    write_json(
        &root.join("edge_proxy_confidence.json"),
        &json!({
            "rejected_code": err.code(),
            "valid_proxy_confidence": ok.confidence,
            "valid_proxy_source": ok.source,
        }),
    );
}

/// Happy path: proxy-grounded diagnostic is Provisional and a provisional-only forecast is refused;
/// the same panel grounded on the resolved outcome is Trusted and the forecast becomes admissible.
/// Both TrustTag states are proven from disk, and the backfill transition is persisted.
fn happy_proxy_then_resolved_lifecycle(root: &Path, clock: &FixedClock) {
    // Same feature panel; only the grounding anchors differ (proxy vs resolved).
    let base = independent(70_209, 170, 3);

    // 1) Grounded on PROXY anchors on a still-open market → Provisional.
    let proxy_anchors: Vec<Anchor> = (0..base.anchors.len())
        .map(|i| proxy_anchor(ProxyKind::Up24h, i % 2 == 0, 0.6, i as u64).expect("proxy"))
        .collect();
    let proxy_matrix = PanelMatrix::new(
        base.keys.clone(),
        base.columns.clone(),
        proxy_anchors.clone(),
    )
    .expect("matrix");
    let proxy_diag =
        compute_panel_diagnostics("open_market", PANEL_VERSION, &proxy_matrix, clock, &cfg())
            .expect("compute proxy diag");
    assert_eq!(
        proxy_diag.trust,
        TrustTag::Provisional,
        "a proxy-grounded diagnostic must be Provisional even above the sample floor"
    );
    let proxy_path = write_panel_diagnostics(root, &proxy_diag).expect("write proxy diag");
    assert_eq!(
        read_panel_diagnostics(&proxy_path).expect("read").trust,
        TrustTag::Provisional,
        "on-disk proxy record must read back Provisional"
    );

    // The admission guard refuses a forecast whose load-bearing evidence is provisional-only.
    let refusal = refuse_if_provisional_only(&[proxy_diag.trust]).expect("must refuse");
    assert_eq!(refusal.code, REFUSE_PROVISIONAL_ONLY);
    assert!(!refusal.admitted);
    let refused = admit_forecast(
        &AdmissionParams::default(),
        &healthy_inputs(),
        &[proxy_diag.trust],
    );
    assert_eq!(
        refused.code, REFUSE_PROVISIONAL_ONLY,
        "provisional-only forecast refused"
    );

    // 2) Grounded on the RESOLVED outcome → Trusted.
    let resolved_anchors: Vec<Anchor> = (0..base.anchors.len())
        .map(|i| resolved_anchor(i % 2 == 0, i))
        .collect();
    let resolved_matrix =
        PanelMatrix::new(base.keys.clone(), base.columns.clone(), resolved_anchors)
            .expect("matrix");
    let resolved_diag = compute_panel_diagnostics(
        "resolved_market",
        PANEL_VERSION,
        &resolved_matrix,
        clock,
        &cfg(),
    )
    .expect("compute resolved diag");
    assert_eq!(
        resolved_diag.trust,
        TrustTag::Trusted,
        "resolved grounding → Trusted"
    );
    let resolved_path = write_panel_diagnostics(root, &resolved_diag).expect("write resolved diag");
    assert_eq!(
        read_panel_diagnostics(&resolved_path).expect("read").trust,
        TrustTag::Trusted,
        "on-disk resolved record must read back Trusted"
    );

    // With a Trusted record backing it, the provisional-only guard no longer fires.
    assert!(
        refuse_if_provisional_only(&[resolved_diag.trust]).is_none(),
        "a Trusted record must clear the provisional-only guard"
    );
    let admitted = admit_forecast(
        &AdmissionParams::default(),
        &healthy_inputs(),
        &[resolved_diag.trust],
    );
    assert!(
        admitted.admitted,
        "trusted-backed healthy forecast is admissible: {admitted:?}"
    );

    // 3) The backfill transition (the primitive #77 calls on resolution) upgrades Provisional→Trusted
    //    only through a real resolved anchor, and is persisted as an audit entry.
    let transition = promote_on_resolution(&proxy_anchors[0], &resolved_anchor(true, 0))
        .expect("promotion on matching resolution");
    assert_eq!(transition.from, TrustTag::Provisional);
    assert_eq!(transition.to, TrustTag::Trusted);
    write_json(
        &root.join("trust_transition.json"),
        &serde_json::to_value(&transition).expect("serialize transition"),
    );

    write_json(
        &root.join("lifecycle_summary.json"),
        &json!({
            "proxy_record_path": proxy_path.display().to_string(),
            "proxy_trust": format!("{:?}", proxy_diag.trust),
            "resolved_record_path": resolved_path.display().to_string(),
            "resolved_trust": format!("{:?}", resolved_diag.trust),
            "provisional_only_refusal_code": refused.code,
            "admitted_after_resolution": admitted.admitted,
        }),
    );
}

/// Edge: a resolution that contradicts the proxy prediction fails closed — no silent promotion of a
/// wrong estimate.
fn edge_resolution_contradicts_proxy(root: &Path) {
    let proxy = proxy_anchor(ProxyKind::Up24h, true, 0.7, 1).expect("proxy");
    let resolved_no = resolved_anchor(false, 1); // resolved the opposite way
    let err =
        promote_on_resolution(&proxy, &resolved_no).expect_err("contradiction must fail closed");
    assert_eq!(err.code(), ERR_BACKFILL_CONTRADICTION);
    write_json(
        &root.join("edge_contradiction.json"),
        &json!({ "code": err.code(), "message": err.message() }),
    );
}

/// Edge: promoting against a non-resolved (still-proxy) anchor does not promote — grounding must be a
/// resolved outcome. (A backfill for a market that has not resolved yields no promotion.)
fn edge_backfill_unresolved_no_promotion(root: &Path) {
    let proxy = proxy_anchor(ProxyKind::Crossed05, true, 0.55, 1).expect("proxy");
    let still_proxy = proxy_anchor(ProxyKind::Crossed05, true, 0.58, 2).expect("proxy");
    let err =
        promote_on_resolution(&proxy, &still_proxy).expect_err("no resolution → no promotion");
    // grounding_kind_of(still_proxy) is Proxy, so the resolved-anchor requirement fails closed.
    assert!(err.code().starts_with("CALYX_POLY_BACKFILL"));
    assert_eq!(
        grounding_kind_of(&still_proxy).unwrap().trust(),
        TrustTag::Provisional
    );
    write_json(
        &root.join("edge_unresolved.json"),
        &json!({ "code": err.code(), "message": err.message() }),
    );
}

/// A fully-healthy admission input (every quality screen passes) so the only variable under test is
/// the provisional-only trust guard.
fn healthy_inputs() -> AdmissionInputs {
    AdmissionInputs {
        p_win: 0.94,
        confidence: 0.98,
        sufficiency_ok: true,
        evidence_count: 3,
        source_derived_evidence_count: 2,
        stale_evidence_count: 0,
        circular_evidence_count: 0,
        super_intel_pass: true,
        guard_calibrated: true,
        grounding_anchor_count: 50,
        guard_pass: true,
        liquidity_ok: true,
        market_integrity: known_healthy_market_integrity(),
        oracle_risk: known_healthy_oracle_risk(),
        wash_trade: known_healthy_wash_trade(),
        kill_switch_active: false,
        daily_error_score: 0.0,
    }
}

// Silence unused-import lints if the harness feature-gates any screen helper.
#[allow(dead_code)]
fn _assert_screen_type(_s: &MarketIntegrityScreen) {}
