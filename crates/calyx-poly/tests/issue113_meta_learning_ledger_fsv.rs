use std::fs;
use std::path::{Path, PathBuf};

use calyx_poly::{
    ERR_META_LEARNING_INVALID_REQUEST, ERR_META_LEARNING_MISSING_GUARDRAIL,
    ERR_META_LEARNING_MISSING_ROLLBACK, META_LEARNING_LEDGER_FILE, MetaLearningEffect,
    MetaLearningLedgerEntry, MetaLearningLedgerRequest, SelfEvolutionGuardrailRequest,
    SelfEvolutionMetrics, SelfEvolutionTripwires, append_meta_learning_ledger_entry,
    read_meta_learning_ledger_entries, run_self_evolution_guardrail,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const RECORDED_CODE: &str = "CALYX_POLY_META_LEARNING_RECORDED";

#[derive(Clone, Copy)]
enum CaseKind {
    Happy,
    GoodhartRisk,
    MissingGuardrail,
    MissingRollback,
    InvalidEffect,
}

#[test]
fn issue113_meta_learning_ledger_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE113_FSV_ROOT", "poly-issue113-meta-learning");
    reset_dir(&root);

    let happy = run_case(&root, "happy", CaseKind::Happy, RECORDED_CODE);
    let goodhart = run_case(
        &root,
        "goodhart-risk",
        CaseKind::GoodhartRisk,
        RECORDED_CODE,
    );
    let missing_guardrail = run_case(
        &root,
        "missing-guardrail",
        CaseKind::MissingGuardrail,
        ERR_META_LEARNING_MISSING_GUARDRAIL,
    );
    let missing_rollback = run_case(
        &root,
        "missing-rollback",
        CaseKind::MissingRollback,
        ERR_META_LEARNING_MISSING_ROLLBACK,
    );
    let invalid = run_case(
        &root,
        "invalid-effect",
        CaseKind::InvalidEffect,
        ERR_META_LEARNING_INVALID_REQUEST,
    );

    for case in [
        &happy,
        &goodhart,
        &missing_guardrail,
        &missing_rollback,
        &invalid,
    ] {
        assert_eq!(case["ok"], true, "case did not meet expected proof: {case}");
    }

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 113,
        "proof_claim": "Every self-evolution change can append a local meta-learning ledger row that records what changed, why, measured effect, Goodhart/regression flags, guardrail report hash, rollback hash, responsible actor, and FSV artifact pointer; missing guardrail, missing rollback, and malformed effects fail closed before appending.",
        "minimum_sufficient_proof_corpus": {
            "selected": "one clean recorded change, one recorded Goodhart/regression-risk change, one missing guardrail report, one missing rollback artifact, and one non-finite measured effect",
            "why_smaller_would_not_prove": "a clean row proves append/readback, a Goodhart row proves regression flags, and missing guardrail/rollback/malformed effect are distinct #113 audit failure paths",
            "why_larger_would_be_wasteful": "more ledger rows would repeat the same append, hash, JSONL readback, and failure paths without proving a new invariant"
        },
        "source_of_truth": [
            "meta_learning_ledger.jsonl read back from disk",
            "self_evolution_guardrail_report.json read and hashed from disk",
            "rollback.json read and hashed from disk",
            "per-case before.json and after.json file-state readbacks"
        ],
        "happy_path": happy,
        "edge_cases": {
            "goodhart_risk_recorded": goodhart,
            "missing_guardrail": missing_guardrail,
            "missing_rollback": missing_rollback,
            "invalid_effect": invalid
        },
        "physical_files": files
    });
    write_json(&root.join("readback.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue113_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue113_fsv_root={}", root.display());
    }
}

fn run_case(root: &Path, name: &str, kind: CaseKind, expected_code: &str) -> Value {
    let case_dir = root.join(name);
    reset_dir(&case_dir);
    let ledger_dir = case_dir.join("ledger");
    let ledger_path = ledger_dir.join(META_LEARNING_LEDGER_FILE);
    let guardrail = prepare_guardrail(&case_dir);
    let rollback_path = match kind {
        CaseKind::MissingRollback => case_dir.join("missing-rollback.json"),
        _ => guardrail.rollback_path.clone(),
    };
    let guardrail_path = match kind {
        CaseKind::MissingGuardrail => case_dir.join("missing-guardrail.json"),
        _ => guardrail.report_path.clone(),
    };

    let before = state(&ledger_path, &guardrail_path, &rollback_path);
    write_json(&case_dir.join("before.json"), &before);
    let fsv_artifact_path = case_dir.join("readback.json");
    let request = MetaLearningLedgerRequest {
        ledger_dir: &ledger_dir,
        change_id: "issue113-change",
        changed_surface: "self_evolution_guardrails",
        rationale: "known-truth meta-learning ledger FSV row",
        responsible_actor: "calyx-poly-fsv",
        effect: effect(kind),
        guardrail_report_path: &guardrail_path,
        rollback_artifact_path: &rollback_path,
        fsv_artifact_path: &fsv_artifact_path,
    };
    let observed = match append_meta_learning_ledger_entry(&request) {
        Ok(run) => {
            let readback =
                read_meta_learning_ledger_entries(&run.ledger_path).expect("read ledger");
            assert_eq!(readback, run.readback_entries);
            assert_eq!(readback.last(), Some(&run.appended));
            RECORDED_CODE.to_string()
        }
        Err(err) => err.code().to_string(),
    };
    let after = state(&ledger_path, &guardrail_path, &rollback_path);
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

struct GuardrailFixture {
    report_path: PathBuf,
    rollback_path: PathBuf,
}

fn prepare_guardrail(case_dir: &Path) -> GuardrailFixture {
    let guardrail_dir = case_dir.join("guardrail");
    fs::create_dir_all(&guardrail_dir).expect("create guardrail dir");
    let rollback_path = guardrail_dir.join("rollback.json");
    let reproduction_path = guardrail_dir.join("reproduce.ps1");
    fs::write(
        &rollback_path,
        br#"{"restore":"previous self-evolution config"}"#,
    )
    .expect("write rollback");
    fs::write(
        &reproduction_path,
        "cargo test -p calyx-poly --test __calyx_integration_suite_1 issue113_meta_learning_ledger_fsv\n",
    )
    .expect("write reproduction");
    let request = SelfEvolutionGuardrailRequest {
        out_dir: &guardrail_dir,
        change_id: "issue113-guardrail",
        rationale: "meta-learning ledger fixture",
        baseline: SelfEvolutionMetrics {
            kernel_recall_ratio: 0.970,
            guard_far_ratio: 0.010,
            p95_latency_ms: 120.0,
        },
        candidate: SelfEvolutionMetrics {
            kernel_recall_ratio: 0.972,
            guard_far_ratio: 0.009,
            p95_latency_ms: 115.0,
        },
        tripwires: SelfEvolutionTripwires {
            min_kernel_recall_ratio: 0.950,
            max_recall_regression: 0.010,
            max_guard_far_ratio: 0.030,
            max_guard_far_increase: 0.010,
            max_p95_latency_ms: 200.0,
            max_latency_increase_ratio: 1.25,
        },
        rollback_artifact_path: &rollback_path,
        reproduction_plan_path: &reproduction_path,
    };
    let run = run_self_evolution_guardrail(&request).expect("write guardrail fixture");
    GuardrailFixture {
        report_path: run.report_path,
        rollback_path,
    }
}

fn effect(kind: CaseKind) -> MetaLearningEffect {
    match kind {
        CaseKind::GoodhartRisk => MetaLearningEffect {
            objective_score_delta: 0.08,
            kernel_recall_delta: -0.04,
            guard_far_delta: 0.04,
            p95_latency_delta_ms: 140.0,
        },
        CaseKind::InvalidEffect => MetaLearningEffect {
            objective_score_delta: f64::NAN,
            kernel_recall_delta: 0.0,
            guard_far_delta: 0.0,
            p95_latency_delta_ms: 0.0,
        },
        _ => MetaLearningEffect {
            objective_score_delta: 0.03,
            kernel_recall_delta: 0.002,
            guard_far_delta: -0.001,
            p95_latency_delta_ms: -5.0,
        },
    }
}

fn state(ledger_path: &Path, guardrail_path: &Path, rollback_path: &Path) -> Value {
    json!({
        "ledger": ledger_state(ledger_path),
        "guardrail": file_state(guardrail_path),
        "rollback": file_state(rollback_path)
    })
}

fn ledger_state(path: &Path) -> Value {
    let bytes = fs::read(path).ok();
    let entries = read_meta_learning_ledger_entries(path).unwrap_or_default();
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
        "blake3": bytes.as_ref().map(|b| hex(blake3::hash(b).as_bytes())),
        "entry_count": entries.len(),
        "last_entry": entries.last().map(entry_summary)
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

fn entry_summary(entry: &MetaLearningLedgerEntry) -> Value {
    json!({
        "sequence": entry.sequence,
        "change_id": &entry.change_id,
        "goodhart_risk": entry.goodhart_risk,
        "regression_flags": &entry.regression_flags,
        "guardrail_status": &entry.guardrail_status,
        "rollback_artifact_blake3": &entry.rollback_artifact_blake3,
        "guardrail_report_blake3": &entry.guardrail_report_blake3,
        "entry_blake3": &entry.entry_blake3
    })
}
