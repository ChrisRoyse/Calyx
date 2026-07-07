//! Issue #96 - end-to-end forecast admission and provenance ledger FSV.
//!
//! Source of truth: local source/association/agent/forecast artifacts plus real AsterVault Ledger
//! rows, all read back independently.

use serde_json::json;

#[path = "issue096/static.rs"]
mod issue096_static;
#[path = "issue096/support.rs"]
mod issue096_support;
#[path = "fsv_support.rs"]
mod support;

use issue096_support::{
    edge_forbidden_trading_instruction_refuses_without_admitted_row,
    edge_malformed_llm_output_refuses_without_admitted_row,
    edge_missing_source_refuses_without_admitted_row, happy_end_to_end_admits_with_full_provenance,
    setup_fixture,
};
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue096_forecast_admission_provenance_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE096_FSV_ROOT", "poly-issue096-provenance");
    reset_dir(&root);

    let fixture = setup_fixture(&root.join("vault"));
    let happy = happy_end_to_end_admits_with_full_provenance(&root, &fixture);
    let missing_source = edge_missing_source_refuses_without_admitted_row(&root, &fixture);
    let malformed_llm = edge_malformed_llm_output_refuses_without_admitted_row(&root, &fixture);
    let forbidden =
        edge_forbidden_trading_instruction_refuses_without_admitted_row(&root, &fixture);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 96,
        "proof_claim": "One local forecast can move from a read-only source snapshot through a local association artifact, optional DeepSeek-style agent artifacts, policy/admission checks, durable forecast JSON/markdown, and an Aster admission ledger row; missing provenance, malformed LLM output, and forbidden trading instructions refuse without admitted rows.",
        "minimum_sufficient_corpus": {
            "source_snapshots": 1,
            "association_artifacts": 1,
            "agent_responses": 1,
            "admission_ledger_rows": 1,
            "edge_cases": 3,
            "why_this_is_sufficient": "A single source snapshot, association artifact, agent response, forecast artifact pair, policy decision, and ledger row exercise every #96 provenance link once; one edge each proves the named refusal paths.",
            "why_smaller_is_insufficient": "Without the source snapshot, association artifact, agent artifacts, forecast files, policy decision, or ledger row, one required provenance link would be unproven.",
            "why_larger_is_wasteful": "More snapshots or agent responses would repeat the same artifact hashing, readback, admission, and ledger paths without adding proof; scale is not the #96 claim."
        },
        "happy_path": happy,
        "edge_cases": {
            "missing_source": missing_source,
            "malformed_llm_output": malformed_llm,
            "forbidden_trading_instruction": forbidden
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE096_FORECAST_ADMISSION_PROVENANCE_READBACK={}",
        readback_path.display()
    );
}
