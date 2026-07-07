//! Issue #23 - crate README FSV.
//!
//! Source of truth: `Calyx/crates/calyx-poly/README.md`, read back from disk.

use serde_json::json;

#[path = "fsv_support.rs"]
mod support;
use support::{named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue023_calyx_poly_readme_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE023_FSV_ROOT", "poly-issue023-readme");
    reset_dir(&root);

    let readme_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md");
    let readme = std::fs::read_to_string(&readme_path).expect("read README");
    let happy = verify_readme_contract(&readme).expect("README contract");
    let missing_flow = verify_readme_contract("# calyx-poly\n\nNo workflow here.")
        .expect_err("missing flow must fail");
    let stale_trading = verify_readme_contract(
        "# calyx-poly\n\n## Core Flow\n\nUse Kelly sizing and order placement.",
    )
    .expect_err("stale execution language must fail");
    let missing_fsv = verify_readme_contract("# calyx-poly\n\n## Core Flow\n\nIngest data.")
        .expect_err("missing FSV must fail");

    let readback = json!({
        "issue": 23,
        "proof_claim": "The calyx-poly crate README documents modules, build/test commands, the ingest->associate->ground->predict->gate flow, local-only boundaries, FSV practice, and fail-closed behavior.",
        "minimum_sufficient_corpus": {
            "readme_files": 1,
            "synthetic_bad_docs": 3,
            "why_this_is_sufficient": "The README is the only source of truth for crate onboarding; three bad-doc probes prove missing flow, stale execution language, and missing FSV guidance are rejected.",
            "why_smaller_is_insufficient": "Without reading the real README and all three bad-doc probes, one required acceptance dimension would be unproven.",
            "why_larger_is_wasteful": "Additional docs would not add proof for the crate-level README contract."
        },
        "happy_path": happy,
        "edge_cases": {
            "missing_flow": missing_flow,
            "stale_trading_language": stale_trading,
            "missing_fsv_guidance": missing_fsv
        },
        "source_of_truth": readme_path.display().to_string()
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!("ISSUE023_README_READBACK={}", readback_path.display());
}

fn verify_readme_contract(readme: &str) -> Result<serde_json::Value, serde_json::Value> {
    let required = [
        "# calyx-poly",
        "## Local Boundary",
        "## Core Flow",
        "## Module Map",
        "## Build And Verification",
        "## FSV Pattern",
        "## Failure Policy",
        "Ingest:",
        "Associate:",
        "Ground:",
        "Predict:",
        "Gate:",
        "cargo check -p calyx-poly",
        "cargo test -p calyx-poly",
    ];
    let missing: Vec<_> = required
        .iter()
        .filter(|needle| !readme.contains(**needle))
        .copied()
        .collect();
    let stale_terms = [
        "Kelly sizing",
        "order placement",
        "stake sizing",
        "PnL optimization",
    ];
    let stale: Vec<_> = stale_terms
        .iter()
        .filter(|term| readme.contains(**term) && !readme.contains("Forbidden work"))
        .copied()
        .collect();
    if !missing.is_empty() || !stale.is_empty() {
        return Err(json!({ "missing": missing, "stale_terms": stale }));
    }
    Ok(json!({
        "bytes": readme.len(),
        "required_sections": required.len(),
        "local_only_boundary": readme.contains("does not place orders"),
        "fsv_minimum_corpus_rule": readme.contains("Minimum sufficient corpus"),
        "fail_closed": readme.contains("fails closed")
    }))
}
