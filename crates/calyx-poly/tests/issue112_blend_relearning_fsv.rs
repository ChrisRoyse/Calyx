use std::fs;
use std::path::Path;

use calyx_poly::{
    BLEND_RELEARNING_REPORT_FILE, BlendRelearningReport, BlendRelearningRequest,
    BlendWeightObservation, ComponentKind, ERR_BLEND_RELEARNING_EMPTY,
    ERR_BLEND_RELEARNING_INSUFFICIENT, ERR_BLEND_RELEARNING_INVALID, ERR_BLEND_RELEARNING_NO_SKILL,
    read_blend_relearning_report, run_blend_relearning,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const AS_OF: u64 = 1_785_400_000_000;
const HEALTHY_CODE: &str = "CALYX_POLY_BLEND_RELEARNING_WRITTEN";

#[derive(Clone, Copy)]
enum CaseKind {
    Happy,
    Empty,
    Insufficient,
    NonFinite,
    NoSkill,
}

#[test]
fn issue112_blend_relearning_fsv() {
    let (root, keep_root) =
        named_fsv_root("POLY_ISSUE112_FSV_ROOT", "poly-issue112-blend-relearning");
    reset_dir(&root);

    let happy = run_case(&root, "happy", CaseKind::Happy, HEALTHY_CODE);
    let empty = run_case(&root, "empty", CaseKind::Empty, ERR_BLEND_RELEARNING_EMPTY);
    let insufficient = run_case(
        &root,
        "insufficient",
        CaseKind::Insufficient,
        ERR_BLEND_RELEARNING_INSUFFICIENT,
    );
    let nonfinite = run_case(
        &root,
        "non-finite",
        CaseKind::NonFinite,
        ERR_BLEND_RELEARNING_INVALID,
    );
    let no_skill = run_case(
        &root,
        "no-skill",
        CaseKind::NoSkill,
        ERR_BLEND_RELEARNING_NO_SKILL,
    );

    for case in [&happy, &empty, &insufficient, &nonfinite, &no_skill] {
        assert_eq!(case["ok"], true, "case did not meet expected proof: {case}");
    }

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 112,
        "proof_claim": "Poly relearns blend component reliability weights from rolling held-out Brier, persists the report, reads it back, and fails closed on empty, insufficient, non-finite, or no-skill observation corpora.",
        "minimum_sufficient_proof_corpus": {
            "selected": "two components with one resolved YES and one resolved NO each, plus four one-purpose edge corpora",
            "why_smaller_would_not_prove": "one row per component would not prove Brier against both outcome classes; fewer than two components would not prove ensemble weight relearning",
            "why_larger_would_be_wasteful": "additional held-out rows would repeat the same Brier, reliability, normalization, persistence, and readback paths without adding proof"
        },
        "source_of_truth": [
            "blend_relearning_report.json read back from disk",
            "per-case before.json and after.json file-state readbacks"
        ],
        "happy_path": happy,
        "edge_cases": {
            "empty": empty,
            "insufficient": insufficient,
            "non_finite": nonfinite,
            "no_skill": no_skill
        },
        "physical_files": files
    });
    write_json(&root.join("readback.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue112_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue112_fsv_root={}", root.display());
    }
}

fn run_case(root: &Path, name: &str, kind: CaseKind, expected_code: &str) -> Value {
    let case_dir = root.join(name);
    reset_dir(&case_dir);
    let report_path = case_dir.join(BLEND_RELEARNING_REPORT_FILE);
    let before = state(&report_path);
    write_json(&case_dir.join("before.json"), &before);

    let request = BlendRelearningRequest {
        out_dir: &case_dir,
        domain: "crypto",
        horizon_bucket: "1h_24h",
        as_of_millis: AS_OF,
        min_samples_per_component: 2,
        observations: observations(kind),
    };
    let observed = match run_blend_relearning(&request) {
        Ok(run) => {
            let readback = read_blend_relearning_report(&run.report_path).expect("read report");
            assert_eq!(readback, run.report, "report must round-trip exactly");
            assert_eq!(readback.component_count, 2);
            assert!(readback.total_reliability_weight > 0.0);
            HEALTHY_CODE.to_string()
        }
        Err(err) => err.code().to_string(),
    };

    let after = state(&report_path);
    write_json(&case_dir.join("after.json"), &after);
    let outcome = json!({
        "case": name,
        "expected_code": expected_code,
        "observed_code": observed,
        "ok": observed == expected_code,
        "before": before,
        "after": after
    });
    write_json(&case_dir.join("outcome.json"), &outcome);
    outcome
}

fn observations(kind: CaseKind) -> Vec<BlendWeightObservation> {
    match kind {
        CaseKind::Empty => Vec::new(),
        CaseKind::Insufficient => vec![
            obs(ComponentKind::KnnBaseRate, 0.90, true, 1),
            obs(ComponentKind::Oracle, 0.10, true, 2),
        ],
        CaseKind::NonFinite => vec![
            obs(ComponentKind::KnnBaseRate, f64::NAN, true, 1),
            obs(ComponentKind::KnnBaseRate, 0.10, false, 2),
            obs(ComponentKind::Oracle, 0.10, true, 3),
            obs(ComponentKind::Oracle, 0.90, false, 4),
        ],
        CaseKind::NoSkill => vec![
            obs(ComponentKind::KnnBaseRate, 0.50, true, 1),
            obs(ComponentKind::KnnBaseRate, 0.50, false, 2),
            obs(ComponentKind::Oracle, 0.50, true, 3),
            obs(ComponentKind::Oracle, 0.50, false, 4),
        ],
        CaseKind::Happy => vec![
            obs(ComponentKind::KnnBaseRate, 0.90, true, 1),
            obs(ComponentKind::KnnBaseRate, 0.10, false, 2),
            obs(ComponentKind::Oracle, 0.10, true, 3),
            obs(ComponentKind::Oracle, 0.90, false, 4),
        ],
    }
}

fn obs(
    component: ComponentKind,
    p_yes: f64,
    outcome_yes: bool,
    offset: u64,
) -> BlendWeightObservation {
    BlendWeightObservation {
        component,
        p_yes,
        outcome_yes,
        observed_at_millis: AS_OF - 10_000 + offset,
    }
}

fn state(report_path: &Path) -> Value {
    let bytes = fs::read(report_path).ok();
    let readback = bytes
        .as_ref()
        .and_then(|b| serde_json::from_slice::<BlendRelearningReport>(b).ok());
    json!({
        "path": report_path.display().to_string(),
        "exists": report_path.exists(),
        "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
        "blake3": bytes.as_ref().map(|b| hex(blake3::hash(b).as_bytes())),
        "readback": readback.as_ref().map(report_summary)
    })
}

fn report_summary(report: &BlendRelearningReport) -> Value {
    json!({
        "component_count": report.component_count,
        "observation_count": report.observation_count,
        "total_reliability_weight": report.total_reliability_weight,
        "rows": report.rows.iter().map(|row| {
            json!({
                "component": row.component.slug(),
                "n": row.n,
                "brier": row.brier,
                "reliability_weight": row.reliability_weight,
                "normalized_weight": row.normalized_weight
            })
        }).collect::<Vec<_>>()
    })
}
