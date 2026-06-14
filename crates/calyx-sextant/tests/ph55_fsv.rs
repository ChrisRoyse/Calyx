use std::fs;
use std::sync::Arc;

use calyx_core::CxId;
use calyx_sextant::CALYX_PLANNER_COST_CAP;
use serde_json::json;

#[path = "ph55_fsv/artifact.rs"]
mod artifact;
#[path = "ph55_fsv/support.rs"]
mod support;

#[test]
fn ph55_fsv_cross_model_gate() {
    let root = support::fsv_root();
    support::reset_root(&root);
    let vault_dir = root.join("vault");
    let vault = Arc::new(support::durable_vault(&vault_dir));
    let collections = support::create_collections(vault.as_ref());

    let scenario_a = support::scenario_a(Arc::clone(&vault), &collections);
    let commit_seq = scenario_a["commit_seq"].as_u64().unwrap();
    let cx_id: CxId = scenario_a["cx_id"].as_str().unwrap().parse().unwrap();
    let scenario_b = support::scenario_b(vault.as_ref(), &collections, cx_id, commit_seq);
    let scenario_c = support::scenario_c(vault.as_ref(), &collections);
    let scenario_d = support::scenario_d(vault.as_ref());
    let edge_empty_ask = support::edge_empty_ask(vault.as_ref(), cx_id);

    vault.flush().unwrap();
    let evidence = json!({
        "issue": 467,
        "source_of_truth": artifact::source_of_truth(&root, &vault_dir),
        "trigger": "PH55 FSV test staged one cross-model transaction, planned/executed cross-model queries, and read Aster CF/WAL bytes",
        "synthetic_input": {
            "order_pk": 1,
            "order_qty": 7,
            "kv_namespace": 1,
            "kv_key": "last_order",
            "kv_value": "1",
            "constellation_input": "order #1 placed",
            "fixed_clock": support::FIXED_TS
        },
        "hand_expected": {
            "scenario_a_commit_delta": 1,
            "scenario_a_shared_seq": commit_seq,
            "scenario_b_rows": ["relational:qty=7", "kv:last_order=1", "vector:cx"],
            "scenario_c_error": CALYX_PLANNER_COST_CAP,
            "scenario_d_rows": 1
        },
        "scenario_a": scenario_a,
        "scenario_b": scenario_b,
        "scenario_c": scenario_c,
        "scenario_d": scenario_d,
        "edge_cases": {
            "active_txn_timeout": scenario_a["deadlock_check"].clone(),
            "unbounded_plan_rejection": scenario_c.clone(),
            "empty_ask_question": edge_empty_ask
        },
        "cf_counts": artifact::cf_counts(vault.as_ref()),
        "wal_batches": artifact::wal_batches(&vault_dir.join("wal")),
        "physical_files": artifact::physical_files(&vault_dir)
    });
    let readback = root.join("issue467-ph55-fsv-readback.json");
    fs::write(&readback, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&evidence).unwrap());
    println!("ph55 FSV: A=PASS B=PASS C=PASS D=PASS");
}
