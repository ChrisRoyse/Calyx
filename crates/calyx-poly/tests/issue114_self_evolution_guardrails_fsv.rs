use std::fs;
use std::path::Path;

use calyx_poly::{
    ERR_SELF_EVOLUTION_INVALID_REQUEST, ERR_SELF_EVOLUTION_MISSING_REPRODUCTION,
    ERR_SELF_EVOLUTION_MISSING_ROLLBACK, ERR_SELF_EVOLUTION_TRIPWIRE,
    SELF_EVOLUTION_GUARDRAIL_REPORT_FILE, SelfEvolutionGuardrailReport,
    SelfEvolutionGuardrailRequest, SelfEvolutionMetrics, SelfEvolutionStatus,
    SelfEvolutionTripwires, read_self_evolution_guardrail_report, require_self_evolution_approved,
    run_self_evolution_guardrail,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const HEALTHY_CODE: &str = "CALYX_POLY_SELF_EVOLUTION_APPROVED";

#[derive(Clone, Copy)]
enum CaseKind {
    Happy,
    TripwireRegression,
    MissingRollback,
    MissingReproduction,
    InvalidMetrics,
}

#[test]
fn issue114_self_evolution_guardrails_fsv() {
    let (root, keep_root) =
        named_fsv_root("POLY_ISSUE114_FSV_ROOT", "poly-issue114-self-evolution");
    reset_dir(&root);

    let happy = run_case(&root, "happy", CaseKind::Happy, HEALTHY_CODE);
    let regression = run_case(
        &root,
        "tripwire-regression",
        CaseKind::TripwireRegression,
        ERR_SELF_EVOLUTION_TRIPWIRE,
    );
    let missing_rollback = run_case(
        &root,
        "missing-rollback",
        CaseKind::MissingRollback,
        ERR_SELF_EVOLUTION_MISSING_ROLLBACK,
    );
    let missing_reproduction = run_case(
        &root,
        "missing-reproduction",
        CaseKind::MissingReproduction,
        ERR_SELF_EVOLUTION_MISSING_REPRODUCTION,
    );
    let invalid = run_case(
        &root,
        "invalid-metrics",
        CaseKind::InvalidMetrics,
        ERR_SELF_EVOLUTION_INVALID_REQUEST,
    );

    for case in [
        &happy,
        &regression,
        &missing_rollback,
        &missing_reproduction,
        &invalid,
    ] {
        assert_eq!(case["ok"], true, "case did not meet expected proof: {case}");
    }

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 114,
        "proof_claim": "A self-evolution candidate is approved only when kernel recall, guard-FAR, and p95 latency clear tripwires and physical rollback plus reproduction artifacts are read and hashed; regressions, missing artifacts, and malformed metrics fail closed.",
        "minimum_sufficient_proof_corpus": {
            "selected": "one approved candidate with two physical artifacts, one rejected candidate that fails recall/guard/latency checks, one missing rollback artifact, one missing reproduction artifact, and one non-finite metrics request",
            "why_smaller_would_not_prove": "recall, guard-FAR, latency, rollback, reproduction, and malformed-input checks are separate #114 invariants; omitting one leaves a required guard unproven",
            "why_larger_would_be_wasteful": "additional candidate rows would repeat the same tripwire, artifact hashing, report write, and readback paths without adding proof"
        },
        "source_of_truth": [
            "rollback.json files read and hashed from disk",
            "reproduce.ps1 files read and hashed from disk",
            "self_evolution_guardrail_report.json read back from disk",
            "per-case before.json and after.json file-state readbacks"
        ],
        "happy_path": happy,
        "edge_cases": {
            "tripwire_regression": regression,
            "missing_rollback": missing_rollback,
            "missing_reproduction": missing_reproduction,
            "invalid_metrics": invalid
        },
        "physical_files": files
    });
    write_json(&root.join("readback.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue114_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue114_fsv_root={}", root.display());
    }
}

fn run_case(root: &Path, name: &str, kind: CaseKind, expected_code: &str) -> Value {
    let case_dir = root.join(name);
    reset_dir(&case_dir);
    let rollback_path = case_dir.join("rollback.json");
    let reproduction_path = case_dir.join("reproduce.ps1");
    let report_path = case_dir.join(SELF_EVOLUTION_GUARDRAIL_REPORT_FILE);
    if !matches!(kind, CaseKind::MissingRollback) {
        fs::write(
            &rollback_path,
            br#"{"restore":"previous parameters","change":"issue114"}"#,
        )
        .expect("write rollback artifact");
    }
    if !matches!(kind, CaseKind::MissingReproduction) {
        fs::write(
            &reproduction_path,
            "cargo test -p calyx-poly --test __calyx_integration_suite_0 issue114_self_evolution_guardrails_fsv\n",
        )
        .expect("write reproduction plan");
    }

    let before = state(&case_dir, &rollback_path, &reproduction_path, &report_path);
    write_json(&case_dir.join("before.json"), &before);
    let request = request(&case_dir, &rollback_path, &reproduction_path, kind);
    let observed = match run_self_evolution_guardrail(&request) {
        Ok(run) => {
            let readback =
                read_self_evolution_guardrail_report(&run.report_path).expect("read report");
            assert_eq!(readback, run.report, "report must round-trip exactly");
            match require_self_evolution_approved(&readback) {
                Ok(()) => HEALTHY_CODE.to_string(),
                Err(err) => err.code().to_string(),
            }
        }
        Err(err) => err.code().to_string(),
    };
    let after = state(&case_dir, &rollback_path, &reproduction_path, &report_path);
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

fn request<'a>(
    out_dir: &'a Path,
    rollback_path: &'a Path,
    reproduction_path: &'a Path,
    kind: CaseKind,
) -> SelfEvolutionGuardrailRequest<'a> {
    SelfEvolutionGuardrailRequest {
        out_dir,
        change_id: "issue114-change",
        rationale: "known-truth self-evolution guardrail FSV candidate",
        baseline: SelfEvolutionMetrics {
            kernel_recall_ratio: 0.970,
            guard_far_ratio: 0.010,
            p95_latency_ms: 120.0,
        },
        candidate: candidate_metrics(kind),
        tripwires: SelfEvolutionTripwires {
            min_kernel_recall_ratio: 0.950,
            max_recall_regression: 0.010,
            max_guard_far_ratio: 0.030,
            max_guard_far_increase: 0.010,
            max_p95_latency_ms: 200.0,
            max_latency_increase_ratio: 1.25,
        },
        rollback_artifact_path: rollback_path,
        reproduction_plan_path: reproduction_path,
    }
}

fn candidate_metrics(kind: CaseKind) -> SelfEvolutionMetrics {
    match kind {
        CaseKind::TripwireRegression => SelfEvolutionMetrics {
            kernel_recall_ratio: 0.920,
            guard_far_ratio: 0.070,
            p95_latency_ms: 500.0,
        },
        CaseKind::InvalidMetrics => SelfEvolutionMetrics {
            kernel_recall_ratio: f64::NAN,
            guard_far_ratio: 0.010,
            p95_latency_ms: 120.0,
        },
        _ => SelfEvolutionMetrics {
            kernel_recall_ratio: 0.971,
            guard_far_ratio: 0.011,
            p95_latency_ms: 122.0,
        },
    }
}

fn state(
    case_dir: &Path,
    rollback_path: &Path,
    reproduction_path: &Path,
    report_path: &Path,
) -> Value {
    json!({
        "case_dir_exists": case_dir.exists(),
        "rollback": file_state(rollback_path),
        "reproduction": file_state(reproduction_path),
        "report": report_state(report_path)
    })
}

fn file_state(path: &Path) -> Value {
    let bytes = fs::read(path).ok();
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
        "blake3": bytes.as_ref().map(|b| hex(blake3::hash(b).as_bytes()))
    })
}

fn report_state(path: &Path) -> Value {
    let bytes = fs::read(path).ok();
    let readback = bytes
        .as_ref()
        .and_then(|b| serde_json::from_slice::<SelfEvolutionGuardrailReport>(b).ok());
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
        "blake3": bytes.as_ref().map(|b| hex(blake3::hash(b).as_bytes())),
        "readback": readback.as_ref().map(report_summary)
    })
}

fn report_summary(report: &SelfEvolutionGuardrailReport) -> Value {
    json!({
        "status": match report.status {
            SelfEvolutionStatus::Approved => "approved",
            SelfEvolutionStatus::Rejected => "rejected",
        },
        "failed_check_count": report.failed_check_count,
        "reversible": report.reversible,
        "reproducible": report.reproducible,
        "rollback_artifact_blake3": &report.rollback_artifact_blake3,
        "reproduction_plan_blake3": &report.reproduction_plan_blake3,
        "checks": report.checks.iter().map(|check| {
            json!({
                "name": &check.name,
                "candidate": check.candidate,
                "comparator": &check.comparator,
                "threshold": check.threshold,
                "passed": check.passed
            })
        }).collect::<Vec<_>>()
    })
}
