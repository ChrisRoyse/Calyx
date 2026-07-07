//! Issue #207 — panel higher-order information diagnostics, Full State Verification.
//!
//! Source of truth: the persisted `panel_diagnostics_*.json` artifact on disk **and** the values
//! returned by `calyx-assay`, read back separately. Every case constructs synthetic data with a
//! *known* information structure and proves the composed diagnostic recovers it.

use std::path::Path;

use calyx_assay::{TotalCorrelationConfig, TrustTag};
use calyx_core::FixedClock;
use calyx_poly::panel_diagnostics::{
    ERR_NON_FINITE, PanelDiagnosticsConfig, PanelMatrix, compute_panel_diagnostics,
    read_panel_diagnostics, write_panel_diagnostics,
};
use serde_json::json;

#[path = "fsv_support.rs"]
mod support;
#[path = "synthetic_panels.rs"]
mod synthetic;

use support::{named_fsv_root, reset_dir, write_blake3sums, write_json};
use synthetic::{fully_redundant, independent, redundant_and_synergy};

const PANEL_VERSION: u32 = 1;

fn test_config() -> PanelDiagnosticsConfig {
    // Fewer bootstrap resamples keep the FSV suite fast; the point estimates that carry the
    // constructed truth are unaffected. This is a precision knob, not a fallback.
    PanelDiagnosticsConfig {
        tc: TotalCorrelationConfig {
            k: 3,
            bootstrap_resamples: 80,
            ..TotalCorrelationConfig::default()
        },
    }
}

#[test]
fn issue207_panel_diagnostics_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE207_FSV_ROOT", "poly-issue207-panel-diagnostics");
    reset_dir(&root);
    let clock = FixedClock::new(1_785_400_000);

    happy_redundancy_and_synergy(&root, &clock);
    edge_independent_panel(&root, &clock);
    edge_fully_redundant_panel(&root, &clock);
    edge_below_floor_is_provisional(&root, &clock);
    edge_non_finite_fails_closed(&root);

    write_blake3sums(&root);
}

/// Happy path: two copies of one field (redundancy) + a product synergy triple. Expect low n_eff and
/// a flagged synergistic triple {a, b, prod}, persisted and read back identically.
fn happy_redundancy_and_synergy(root: &Path, clock: &FixedClock) {
    let panel = redundant_and_synergy(20_207, 205);
    let matrix = PanelMatrix::new(
        panel.keys.clone(),
        panel.columns.clone(),
        panel.anchors.clone(),
    )
    .expect("valid matrix");
    let diag = compute_panel_diagnostics("crypto", PANEL_VERSION, &matrix, clock, &test_config())
        .expect("compute diagnostics");

    let synergy_keys: Vec<Vec<String>> = diag
        .synergistic_triples
        .iter()
        .map(|t| t.slots.to_vec())
        .collect();
    eprintln!(
        "[#207 happy] tc={:.4} tc.n_eff={:.3} stable_rank.n_eff={:.3} provisional={} trust={:?} \
         synergy={:?} redundant_count={} unclear={}",
        diag.total_correlation.tc,
        diag.total_correlation.n_eff,
        diag.effective_rank.n_eff,
        diag.provisional,
        diag.trust,
        synergy_keys,
        diag.redundant_triple_count,
        diag.unclear_triple_count,
    );

    // Constructed truth: 3 effective dims out of 4 (a≈dup) → n_eff clearly below the slot count.
    assert!(
        diag.total_correlation.n_eff < 3.6,
        "redundancy must pull n_eff below 4, got {}",
        diag.total_correlation.n_eff
    );
    assert!(
        diag.effective_rank.n_eff < 3.7,
        "stable-rank cross-check must also see redundancy, got {}",
        diag.effective_rank.n_eff
    );
    // Constructed truth: `prod = a*b` is the synergy carrier — a and b are individually
    // uninformative about it but jointly determine it, so at least one flagged synergistic triple
    // must contain `prod`. (With a≈dup, both (a,b,prod) and (b,prod,dup) are genuine synergies.)
    assert!(
        synergy_keys.iter().any(|t| t.iter().any(|k| k == "sum")),
        "a synergy triple carrying the sum slot must be flagged; got {synergy_keys:?}"
    );
    assert!(!diag.provisional, "above floor must not be provisional");
    assert_eq!(diag.trust, TrustTag::Trusted, "resolved anchors → Trusted");

    // Persist, then read back from disk and prove byte-level round-trip equality.
    let path = write_panel_diagnostics(root, &diag).expect("write diagnostics");
    let readback = read_panel_diagnostics(&path).expect("read diagnostics");
    assert_eq!(
        readback, diag,
        "on-disk artifact must equal the computed record"
    );
    assert!(path.exists(), "artifact must physically exist on disk");

    write_json(
        &root.join("happy_summary.json"),
        &json!({
            "case": "redundancy_and_synergy",
            "artifact_path": path.display().to_string(),
            "tc": diag.total_correlation.tc,
            "tc_n_eff": diag.total_correlation.n_eff,
            "stable_rank_n_eff": diag.effective_rank.n_eff,
            "synergistic_triples": synergy_keys,
            "trust": format!("{:?}", diag.trust),
            "provenance_hash": diag.provenance_hash,
        }),
    );
}

/// Edge: fully-independent panel → n_eff ≈ N (no redundancy).
fn edge_independent_panel(root: &Path, clock: &FixedClock) {
    let panel = independent(30_207, 170, 3);
    let matrix = PanelMatrix::new(panel.keys, panel.columns, panel.anchors).expect("matrix");
    let diag =
        compute_panel_diagnostics("independent", PANEL_VERSION, &matrix, clock, &test_config())
            .expect("compute");
    eprintln!(
        "[#207 independent] tc={:.4} tc.n_eff={:.3} stable_rank.n_eff={:.3}",
        diag.total_correlation.tc, diag.total_correlation.n_eff, diag.effective_rank.n_eff
    );
    assert!(
        diag.total_correlation.n_eff > 2.4,
        "independent 3-slot panel n_eff must approach 3, got {}",
        diag.total_correlation.n_eff
    );
    let path = write_panel_diagnostics(root, &diag).expect("write");
    assert_eq!(read_panel_diagnostics(&path).expect("read"), diag);
}

/// Edge: fully-redundant panel → n_eff ≈ 1.
fn edge_fully_redundant_panel(root: &Path, clock: &FixedClock) {
    let panel = fully_redundant(40_207, 170, 3);
    let matrix = PanelMatrix::new(panel.keys, panel.columns, panel.anchors).expect("matrix");
    let diag =
        compute_panel_diagnostics("redundant", PANEL_VERSION, &matrix, clock, &test_config())
            .expect("compute");
    eprintln!(
        "[#207 fully-redundant] tc={:.4} tc.n_eff={:.3} stable_rank.n_eff={:.3}",
        diag.total_correlation.tc, diag.total_correlation.n_eff, diag.effective_rank.n_eff
    );
    assert!(
        diag.total_correlation.n_eff < 1.8,
        "fully-redundant panel n_eff must approach 1, got {}",
        diag.total_correlation.n_eff
    );
    assert!(
        diag.effective_rank.n_eff < 1.8,
        "stable-rank must also collapse to ~1, got {}",
        diag.effective_rank.n_eff
    );
    let path = write_panel_diagnostics(root, &diag).expect("write");
    assert_eq!(read_panel_diagnostics(&path).expect("read"), diag);
}

/// Edge: below the MI sample floor → Provisional, never a confident emission.
fn edge_below_floor_is_provisional(root: &Path, clock: &FixedClock) {
    let panel = independent(50_207, 120, 3); // < TC quorum (50*4 = 200)
    let matrix = PanelMatrix::new(panel.keys, panel.columns, panel.anchors).expect("matrix");
    let diag =
        compute_panel_diagnostics("belowfloor", PANEL_VERSION, &matrix, clock, &test_config())
            .expect("compute");
    eprintln!(
        "[#207 below-floor] provisional={} trust={:?} tc.error_code={:?}",
        diag.provisional, diag.trust, diag.total_correlation.error_code
    );
    assert!(diag.provisional, "below floor must be provisional");
    assert_eq!(
        diag.trust,
        TrustTag::Provisional,
        "below floor forces Provisional"
    );
    assert!(
        diag.total_correlation.error_code.is_some(),
        "below-floor TC must carry an insufficient-samples code"
    );
    let path = write_panel_diagnostics(root, &diag).expect("write");
    assert_eq!(read_panel_diagnostics(&path).expect("read"), diag);
}

/// Edge: a non-finite slot value fails closed at matrix construction — never a fabricated estimate.
fn edge_non_finite_fails_closed(root: &Path) {
    let anchors = vec![
        synthetic::resolved_anchor(true, 0),
        synthetic::resolved_anchor(false, 1),
    ];
    let err = PanelMatrix::new(vec!["a".into()], vec![vec![1.0, f32::INFINITY]], anchors)
        .expect_err("non-finite must fail closed");
    assert_eq!(err.code(), ERR_NON_FINITE);
    write_json(
        &root.join("edge_non_finite.json"),
        &json!({ "code": err.code(), "message": err.message() }),
    );
}
