//! Issue #129 - daily jobs runbook FSV.
//!
//! Source of truth: `docs/poly-daily-jobs-runbook.md`, read back from disk.

use serde_json::json;

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue129_daily_jobs_runbook_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE129_FSV_ROOT", "poly-issue129-daily-jobs");
    reset_dir(&root);

    let runbook_path = doc_path("poly-daily-jobs-runbook.md");
    let runbook = std::fs::read_to_string(&runbook_path).expect("read #129 runbook");
    let happy = verify_runbook_contract(&runbook).expect("daily jobs runbook contract");
    let missing_order = verify_runbook_contract(
        "# Poly Daily Jobs Runbook\n\n## Success Verification\n\nWrite JSON.",
    )
    .expect_err("missing job order must fail");
    let missing_success = verify_runbook_contract(
        "# Poly Daily Jobs Runbook\n\n## Job Order\n\nweave-loom kernel-build guard calibrate verify-chain bits scan.",
    )
    .expect_err("missing success verification must fail");
    let unsafe_retry = verify_runbook_contract(
        "# Poly Daily Jobs Runbook\n\n## Job Order\n\nweave-loom kernel-build guard calibrate verify-chain bits scan.\n\n## Success Verification\n\nIgnore readback mismatch and retry until green.\n\n## Failure Policy\n\nTrading commands are allowed.",
    )
    .expect_err("unsafe retry/trading guidance must fail");

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 129,
        "proof_claim": "The Poly daily jobs runbook schedules weave-loom, kernel-build, guard calibrate, verify-chain, and bits scan with persisted success verification and fail-closed edge handling.",
        "minimum_sufficient_corpus": {
            "runbook_files": 1,
            "synthetic_bad_docs": 3,
            "why_this_is_sufficient": "The runbook is the source of truth for #129; three bad-doc probes prove missing job order, missing success verification, and unsafe retry/trading guidance are rejected.",
            "why_smaller_is_insufficient": "Without the real runbook and all three bad-doc probes, one required scheduler-safety dimension would be unproven.",
            "why_larger_is_wasteful": "Additional host-scale scheduling drills would repeat the same runbook contract checks; this issue asks for the nightly job schedule and success verification contract."
        },
        "happy_path": happy,
        "edge_cases": {
            "missing_job_order": missing_order,
            "missing_success_verification": missing_success,
            "unsafe_retry_or_trading": unsafe_retry
        },
        "source_of_truth": runbook_path.display().to_string(),
        "physical_files_before_readback": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE129_DAILY_JOBS_RUNBOOK_READBACK={}",
        readback_path.display()
    );
}

fn doc_path(name: &str) -> std::path::PathBuf {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let crate_local = crate_root.join("runbooks").join(name);
    if crate_local.exists() {
        return crate_local;
    }
    for ancestor in crate_root.ancestors() {
        let candidate = ancestor.join("docs").join(name);
        if candidate.exists() {
            return candidate;
        }
    }
    crate_root.join("../../docs").join(name)
}

fn verify_runbook_contract(runbook: &str) -> Result<serde_json::Value, serde_json::Value> {
    let required = [
        "# Poly Daily Jobs Runbook",
        "## Scope And Boundary",
        "## Schedule",
        "## Job Order",
        "## Success Verification",
        "## Failure Policy",
        "## Evidence To Save",
        "## Minimum Manual Drill",
        "weave-loom",
        "kernel-build",
        "guard calibrate",
        "verify-chain",
        "bits scan",
        "exclusive lock",
        "readback hash",
        "kernel recall",
        "guard calibration",
        "ledger tip hash",
        "Fail closed",
        "overlapping lock",
        "missing artifact",
        "readback hash mismatch",
    ];
    let missing: Vec<_> = required
        .iter()
        .filter(|needle| !runbook.contains(**needle))
        .copied()
        .collect();
    let unsafe_terms = [
        "Ignore readback mismatch",
        "Trading commands are allowed",
        "retry until green",
    ];
    let unsafe_hits: Vec<_> = unsafe_terms
        .iter()
        .filter(|term| runbook.contains(**term))
        .copied()
        .collect();
    if !missing.is_empty() || !unsafe_hits.is_empty() {
        return Err(json!({"missing": missing, "unsafe_terms": unsafe_hits}));
    }
    Ok(json!({
        "bytes": runbook.len(),
        "blake3": hex(blake3::hash(runbook.as_bytes()).as_bytes()),
        "required_terms": required.len(),
        "job_count": 5,
        "has_success_verification": runbook.contains("every readback hash"),
        "has_fail_closed_edges": runbook.contains("Fail closed on"),
        "local_only_boundary": runbook.contains("must not place trades")
    }))
}
