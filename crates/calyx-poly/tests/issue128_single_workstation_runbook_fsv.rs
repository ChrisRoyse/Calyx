//! Issue #128 - single-workstation runbook and backup FSV.
//!
//! Source of truth: `docs/poly-single-workstation-runbook.md`, read back from disk.

use serde_json::json;

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue128_single_workstation_runbook_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE128_FSV_ROOT", "poly-issue128-runbook");
    reset_dir(&root);

    let runbook_path = doc_path("poly-single-workstation-runbook.md");
    let runbook = std::fs::read_to_string(&runbook_path).expect("read #128 runbook");
    let happy = verify_runbook_contract(&runbook).expect("runbook contract");
    let missing_backup = verify_runbook_contract("# Poly Single-Workstation Runbook\n\nNo backup.")
        .expect_err("missing backup policy must fail");
    let missing_restore = verify_runbook_contract(
        "# Poly Single-Workstation Runbook\n\n## Backup Policy\n\nUse restic.",
    )
    .expect_err("missing restore drill must fail");
    let unsafe_restore = verify_runbook_contract(
        "# Poly Single-Workstation Runbook\n\n## Backup Policy\n\nUse restic and ZFS.\n\n## Restore-Verify Drill\n\nOverwrite the active vault and skip verification.\n\n## Fail-Closed Cases\n\nNone.",
    )
    .expect_err("unsafe restore language must fail");

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 128,
        "proof_claim": "The Poly single-workstation runbook documents local-only startup, restic/ZFS backup policy, restore-verify drill, fail-closed cases, and evidence to save.",
        "minimum_sufficient_corpus": {
            "runbook_files": 1,
            "synthetic_bad_docs": 3,
            "why_this_is_sufficient": "The runbook is the source of truth for #128; three bad-doc probes prove missing backup policy, missing restore verification, and unsafe active-vault restore guidance are rejected.",
            "why_smaller_is_insufficient": "Without reading the real runbook and all three bad-doc probes, one required ops-safety dimension would be unproven.",
            "why_larger_is_wasteful": "Additional docs or host-scale drills would repeat the same runbook contract checks; #128 asks for the workstation runbook and backup/restore-verify procedure."
        },
        "happy_path": happy,
        "edge_cases": {
            "missing_backup_policy": missing_backup,
            "missing_restore_verify": missing_restore,
            "unsafe_restore_guidance": unsafe_restore
        },
        "source_of_truth": runbook_path.display().to_string(),
        "physical_files_before_readback": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE128_SINGLE_WORKSTATION_RUNBOOK_READBACK={}",
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
        "# Poly Single-Workstation Runbook",
        "## Scope And Boundary",
        "## Workstation Layout",
        "## Startup Checklist",
        "## Backup Policy",
        "## Backup Command Template",
        "## Restore-Verify Drill",
        "## Fail-Closed Cases",
        "## Evidence To Save",
        "restic",
        "ZFS",
        "RPO",
        "RTO",
        "calyx verify-restore",
        "Never restore over the active vault/data root",
        "BLAKE3SUMS.txt",
    ];
    let missing: Vec<_> = required
        .iter()
        .filter(|needle| !runbook.contains(**needle))
        .copied()
        .collect();
    let unsafe_terms = [
        "skip verification",
        "Overwrite the active vault",
        "orders, positions, or bankroll material are safe to back up",
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
        "restic": runbook.contains("restic"),
        "zfs": runbook.contains("ZFS"),
        "restore_verify": runbook.contains("Restore-Verify Drill"),
        "fail_closed_cases": runbook.contains("Stop and record evidence"),
        "local_only_boundary": runbook.contains("does not authorize Polymarket")
            && runbook.contains("trading, order signing")
    }))
}
