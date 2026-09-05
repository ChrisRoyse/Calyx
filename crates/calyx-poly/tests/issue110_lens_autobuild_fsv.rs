//! Issue #110 - auto-propose and auto-build lenses from sufficiency deficits.
//!
//! Source of truth: persisted panel-sufficiency and lens-autobuild JSON artifacts, read back from
//! disk before assertions are recorded.

use std::path::Path;

use calyx_assay::{EnsembleConfig, EnsembleLensInput, EstimateBound, TrustTag};
use calyx_core::SlotId;
use calyx_poly::lens_autobuild::{
    BuiltLensSpec, ERR_LENS_AUTOBUILD_NO_ADMISSIBLE, ERR_LENS_AUTOBUILD_NO_CANDIDATES,
    ERR_LENS_AUTOBUILD_NO_DEFICIT, LENS_AUTOBUILD_MIN_GAIN_BITS, LensAutobuildRequest,
    LensAutobuildStatus, LensCandidateMeasurement, LensDeficit, read_lens_autobuild_report,
    require_lens_autobuild_admitted, run_lens_autobuild_report,
};
use calyx_poly::lenses::default_panel;
use calyx_poly::panel_sufficiency::{
    PolyPanelSufficiencyRequest, read_panel_sufficiency_report, run_panel_sufficiency_report,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const MIN_ASSAY_ROWS: usize = 50;
const MIN_PANEL_LENSES: usize = 3;

#[test]
fn issue110_lens_autobuild_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE110_FSV_ROOT", "poly-issue110-lens-autobuild");
    reset_dir(&root);

    let deficit = persisted_propose_lens_deficit(&root);
    let happy = happy_admits_append_lens_spec(&root, &deficit);
    let duplicate = edge_duplicate_lens_fails_closed(&root, &deficit);
    let below_gain = edge_below_gain_fails_closed(&root, &deficit);
    let provisional = edge_provisional_evidence_fails_closed(&root, &deficit);
    let missing_bound = edge_missing_bound_fails_closed(&root, &deficit);
    let forbidden = edge_forbidden_action_fails_closed(&root, &deficit);
    let no_deficit = edge_missing_deficit_fails_loud(&root, &deficit);
    let no_candidates = edge_missing_candidates_fails_loud(&root, &deficit);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 110,
        "proof_claim": "Poly consumes a persisted Assay propose_lens deficit, synthesizes deterministic append_lens_spec artifacts from measured candidate evidence, admits only trusted calibrated-lower-bound lenses above the 0.05-bit floor, and fails closed for duplicate, weak, provisional, unknown-bound, forbidden-action, and structurally incomplete requests.",
        "minimum_sufficient_corpus": {
            "sufficiency_rows": MIN_ASSAY_ROWS,
            "panel_lenses": MIN_PANEL_LENSES,
            "deficit_reports": 1,
            "candidate_measurements": 6,
            "why_this_is_sufficient": "50 rows and 3 lenses are the smallest Assay corpus that can produce a real persisted propose_lens deficit; one accepted candidate and five single-cause rejected candidates prove the append, duplicate, gain-floor, trust, bound, and local-only action gates without conflating rejection causes.",
            "why_smaller_is_insufficient": "49 rows or fewer than 3 lenses cannot produce the source deficit; removing the accepted candidate or any rejected candidate leaves the append, duplicate, gain-floor, trust, bound, or local-only action gate unproven.",
            "why_larger_is_wasteful": "additional rows or candidates would repeat the same deterministic persisted readback and admission gates without proving another #110 behavior."
        },
        "happy_path": happy,
        "edge_cases": {
            "duplicate_existing_lens": duplicate,
            "below_gain_floor": below_gain,
            "provisional_evidence": provisional,
            "missing_estimate_bound": missing_bound,
            "forbidden_action": forbidden,
            "missing_deficit": no_deficit,
            "missing_candidates": no_candidates
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE110_LENS_AUTOBUILD_READBACK={}",
        readback_path.display()
    );
}

fn persisted_propose_lens_deficit(root: &Path) -> LensDeficit {
    let dir = root.join("source-deficit");
    let run = run_panel_sufficiency_report(&noise_request(), &dir).expect("sufficiency run");
    let report = read_panel_sufficiency_report(&run.report_path).expect("read sufficiency report");
    assert_eq!(report, run.report);
    assert!(!report.sufficient);
    assert!(report.has_deficit_proposal);
    LensDeficit::from_panel_sufficiency_report(&report, run.report_path.display().to_string())
        .expect("propose_lens deficit")
}

fn happy_admits_append_lens_spec(root: &Path, deficit: &LensDeficit) -> Value {
    let request = request(
        deficit.clone(),
        vec![candidate(
            root,
            "happy",
            "macro_event_text_bm25",
            0.082,
            0.031,
            TrustTag::Trusted,
            "append_lens_spec",
        )],
    );
    let report = run_and_read(root, "happy", &request);
    require_lens_autobuild_admitted(&report).expect("admitted lens spec");
    assert_eq!(report.status, LensAutobuildStatus::Admitted);
    assert_eq!(report.admitted_count, 1);
    let spec = &report.admitted[0];
    assert_eq!(spec.registry_patch_kind, "append_lens_spec");
    assert_eq!(spec.lens_key, "macro_event_text_bm25");
    assert_eq!(spec.target_slots, deficit.weakest_slots);
    assert_eq!(spec.trust, TrustTag::Trusted);
    assert_eq!(spec.estimate_bound, Some(EstimateBound::LowerBound));
    assert_eq!(spec.lens_id.len(), 64);
    let mut legacy_spec = serde_json::to_value(spec).expect("legacy built lens JSON");
    legacy_spec
        .as_object_mut()
        .expect("built lens object")
        .remove("estimate_bound");
    let legacy_spec: BuiltLensSpec =
        serde_json::from_value(legacy_spec).expect("legacy built lens without bound");
    assert_eq!(legacy_spec.estimate_bound, None);
    json!({
        "status": report.status,
        "admitted_count": report.admitted_count,
        "lens_key": spec.lens_key,
        "target_slots": spec.target_slots,
        "expected_gain_bits": spec.expected_gain_bits,
        "decision_hash": report.decision_hash
    })
}

fn edge_duplicate_lens_fails_closed(root: &Path, deficit: &LensDeficit) -> Value {
    rejected_edge(
        root,
        "edge-duplicate",
        deficit,
        candidate(
            root,
            "edge-duplicate",
            "price_rff",
            0.09,
            0.04,
            TrustTag::Trusted,
            "append_lens_spec",
        ),
    )
}

fn edge_below_gain_fails_closed(root: &Path, deficit: &LensDeficit) -> Value {
    rejected_edge(
        root,
        "edge-below-gain",
        deficit,
        candidate(
            root,
            "edge-below-gain",
            "low_gain_text",
            0.049,
            0.02,
            TrustTag::Trusted,
            "append_lens_spec",
        ),
    )
}

fn edge_provisional_evidence_fails_closed(root: &Path, deficit: &LensDeficit) -> Value {
    rejected_edge(
        root,
        "edge-provisional",
        deficit,
        candidate(
            root,
            "edge-provisional",
            "provisional_text",
            0.09,
            0.04,
            TrustTag::Provisional,
            "append_lens_spec",
        ),
    )
}

fn edge_missing_bound_fails_closed(root: &Path, deficit: &LensDeficit) -> Value {
    let mut uncalibrated = candidate(
        root,
        "edge-missing-bound",
        "unknown_bound_text",
        0.09,
        0.04,
        TrustTag::Trusted,
        "append_lens_spec",
    );
    uncalibrated.estimate_bound = None;
    let report = run_and_read(
        root,
        "edge-missing-bound",
        &request(deficit.clone(), vec![uncalibrated]),
    );
    assert_eq!(report.status, LensAutobuildStatus::Rejected);
    assert_eq!(report.admitted_count, 0);
    let rejection = &report.rejected[0];
    assert_eq!(rejection.code, "uncalibrated_estimate_bound");
    assert_eq!(rejection.estimate_bound, None);
    json!({
        "status": report.status,
        "rejection_code": rejection.code,
        "estimate_bound": rejection.estimate_bound,
        "decision_hash": report.decision_hash
    })
}

fn edge_forbidden_action_fails_closed(root: &Path, deficit: &LensDeficit) -> Value {
    rejected_edge(
        root,
        "edge-forbidden",
        deficit,
        candidate(
            root,
            "edge-forbidden",
            "forbidden_action_text",
            0.09,
            0.04,
            TrustTag::Trusted,
            "submit_order",
        ),
    )
}

fn edge_missing_deficit_fails_loud(root: &Path, deficit: &LensDeficit) -> Value {
    let mut request = request(
        deficit.clone(),
        vec![candidate(
            root,
            "edge-no-deficit",
            "missing_deficit_text",
            0.09,
            0.04,
            TrustTag::Trusted,
            "append_lens_spec",
        )],
    );
    request.deficits.clear();
    let err = run_lens_autobuild_report(&request, &root.join("edge-no-deficit"))
        .expect_err("missing deficit rejected");
    assert_eq!(err.code(), ERR_LENS_AUTOBUILD_NO_DEFICIT);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_missing_candidates_fails_loud(root: &Path, deficit: &LensDeficit) -> Value {
    let request = request(deficit.clone(), Vec::new());
    let err = run_lens_autobuild_report(&request, &root.join("edge-no-candidates"))
        .expect_err("missing candidates rejected");
    assert_eq!(err.code(), ERR_LENS_AUTOBUILD_NO_CANDIDATES);
    json!({"code": err.code(), "message": err.message()})
}

fn rejected_edge(
    root: &Path,
    dir: &str,
    deficit: &LensDeficit,
    candidate: LensCandidateMeasurement,
) -> Value {
    let report = run_and_read(root, dir, &request(deficit.clone(), vec![candidate]));
    assert_eq!(report.status, LensAutobuildStatus::Rejected);
    assert_eq!(report.admitted_count, 0);
    let err = require_lens_autobuild_admitted(&report).expect_err("no admissible lens");
    assert_eq!(err.code(), ERR_LENS_AUTOBUILD_NO_ADMISSIBLE);
    let rejection = &report.rejected[0];
    json!({
        "status": report.status,
        "rejection_code": rejection.code,
        "fail_loud_code": err.code(),
        "lens_key": rejection.lens_key,
        "decision_hash": report.decision_hash
    })
}

fn run_and_read(
    root: &Path,
    dir: &str,
    request: &LensAutobuildRequest,
) -> calyx_poly::LensAutobuildReport {
    let run = run_lens_autobuild_report(request, &root.join(dir)).expect("lens autobuild run");
    let readback = read_lens_autobuild_report(&run.report_path).expect("read lens autobuild");
    assert_eq!(readback, run.report);
    readback
}

fn request(
    deficit: LensDeficit,
    candidates: Vec<LensCandidateMeasurement>,
) -> LensAutobuildRequest {
    LensAutobuildRequest {
        domain: deficit.domain.clone(),
        panel_id: deficit.panel_id.clone(),
        panel_version: deficit.panel_version,
        existing_lens_keys: existing_lens_keys(),
        deficits: vec![deficit],
        candidates,
        min_gain_bits: LENS_AUTOBUILD_MIN_GAIN_BITS,
    }
}

fn candidate(
    root: &Path,
    dir: &str,
    key: &str,
    gain: f32,
    ci_low: f32,
    trust: TrustTag,
    requested_action: &str,
) -> LensCandidateMeasurement {
    let evidence_path = root.join(dir).join(format!("{key}_gain.json"));
    let candidate = LensCandidateMeasurement {
        lens_key: key.to_string(),
        encoder_kind: "bm25_text".to_string(),
        source_fields: vec!["question_text".to_string(), "description_text".to_string()],
        measured_gain_bits: gain,
        ci_low_bits: ci_low,
        ci_high_bits: gain + 0.02,
        n_samples: MIN_ASSAY_ROWS,
        trust,
        estimate_bound: Some(EstimateBound::LowerBound),
        evidence_artifact: evidence_path.display().to_string(),
        requested_action: requested_action.to_string(),
    };
    write_json(
        &evidence_path,
        &serde_json::to_value(&candidate).expect("candidate evidence json"),
    );
    candidate
}

fn existing_lens_keys() -> Vec<String> {
    default_panel(1, vec!["florida".to_string()])
        .lenses
        .iter()
        .map(|lens| lens.key().to_string())
        .collect()
}

fn noise_request() -> PolyPanelSufficiencyRequest {
    let labels = alternating_labels(MIN_ASSAY_ROWS);
    PolyPanelSufficiencyRequest {
        domain: "crypto".to_string(),
        panel_id: "issue110_auto_lens".to_string(),
        panel_version: 1,
        lenses: paired_noise_lenses(MIN_ASSAY_ROWS),
        labels,
        groups: None,
        config: EnsembleConfig {
            source: "issue110_fsv".to_string(),
            min_gate_lenses: MIN_PANEL_LENSES,
            min_marginal_bits: LENS_AUTOBUILD_MIN_GAIN_BITS,
            max_redundancy: 0.95,
            nmi_bins: 8,
        },
    }
}

fn alternating_labels(n: usize) -> Vec<bool> {
    (0..n).map(|idx| idx % 2 == 0).collect()
}

fn paired_noise_lenses(n: usize) -> Vec<EnsembleLensInput> {
    let mut a = Vec::with_capacity(n);
    let mut b = Vec::with_capacity(n);
    let mut c = Vec::with_capacity(n);
    for idx in 0..n {
        let pair = idx / 2;
        a.push(vec![((pair * 17 + 3) % 11) as f32]);
        b.push(vec![((pair * 7 + 5) % 13) as f32]);
        c.push(vec![((pair * 5 + 1) % 17) as f32]);
    }
    vec![
        EnsembleLensInput::new("noise_a", SlotId::new(1), a),
        EnsembleLensInput::new("noise_b", SlotId::new(2), b),
        EnsembleLensInput::new("noise_c", SlotId::new(3), c),
    ]
}
