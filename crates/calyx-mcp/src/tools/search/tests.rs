use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::MutexGuard;

use calyx_core::{AuthN, Modality, SlotState, VaultStore};
use calyx_ward::{
    CalibrationMeta, GuardId, GuardPolicy, GuardProfile, NoveltyAction, SlotCalibrationMeta,
    SlotKind,
};
use serde_json::{Value, json};

use calyx_aster::cf::{CfRouter, ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_registry::load_vault_panel_state;

use crate::jsonrpc::decode_jsonrpc_request;
use crate::protocol::JsonRpcError;
use crate::server::McpServer;
use crate::tools::test_support::ENV_LOCK;

mod support;
use support::*;
#[test]
fn minimal_search_returns_provenanced_hits() {
    let _env = TestEnv::new("minimal");
    let server = server();
    vault_with_algorithmic_data(&server, "v");

    let result = call_ok(
        &server,
        5,
        "calyx.search",
        json!({"vault": "v", "query": "alpha"}),
    );

    let hits = result["hits"].as_array().unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|hit| hit["provenance"].is_object()));
    assert!(hits.iter().all(|hit| hit["per_lens"].is_null()));
}

#[test]
fn search_fails_closed_when_ledger_chain_is_tampered() {
    let env = TestEnv::new("ledger-tamper");
    let server = server();
    let created = call_ok(&server, 1, "calyx.create_vault", json!({"name": "v"}));
    call_ok(
        &server,
        2,
        "calyx.add_lens",
        json!({"vault": "v", "name": "byte_axis", "runtime": "algorithmic"}),
    );
    call_ok(
        &server,
        3,
        "calyx.ingest",
        json!({"vault": "v", "input": "alpha"}),
    );
    let vault_id = created["vault_id"].as_str().unwrap();
    tamper_ledger_row(&env.vault_path(vault_id), 0);

    let error = call_err(
        &server,
        4,
        "calyx.search",
        json!({"vault": "v", "query": "alpha"}),
    );

    assert_eq!(error.code, -32000);
    assert_eq!(
        error.data.unwrap()["calyx_code"],
        "CALYX_LEDGER_CHAIN_BROKEN"
    );
}

#[test]
fn search_explain_includes_per_lens_breakdown() {
    let _env = TestEnv::new("explain");
    let server = server();
    vault_with_algorithmic_data(&server, "v");

    let result = call_ok(
        &server,
        6,
        "calyx.search",
        json!({"vault": "v", "query": "alpha", "explain": true}),
    );

    let first = &result["hits"].as_array().unwrap()[0];
    let per_lens = first["per_lens"].as_array().unwrap();
    assert!(!per_lens.is_empty());
    for field in ["slot", "rank", "raw", "weight", "contribution"] {
        assert!(per_lens[0].get(field).is_some(), "missing {field}");
    }
}

#[test]
fn search_fresh_flag_is_reflected_in_hit_freshness() {
    let _env = TestEnv::new("freshness");
    let server = server();
    vault_with_algorithmic_data(&server, "v");

    let fresh = call_ok(
        &server,
        40,
        "calyx.search",
        json!({"vault": "v", "query": "alpha", "fresh": true}),
    );
    let stale_ok = call_ok(
        &server,
        41,
        "calyx.search",
        json!({"vault": "v", "query": "alpha", "fresh": false}),
    );

    let fresh_hit = &fresh["hits"].as_array().unwrap()[0];
    let stale_hit = &stale_ok["hits"].as_array().unwrap()[0];
    assert_eq!(fresh_hit["freshness"]["policy"], "fresh_derived");
    assert_eq!(fresh_hit["freshness"]["stale_by"], 0);
    assert_eq!(stale_hit["freshness"]["policy"], "stale_ok");
    assert_eq!(stale_hit["freshness"]["stale_by"], 0);
    maybe_write_fsv_json(
        "mcp-search-freshness-readback.json",
        &json!({
            "source_of_truth": "JSON-RPC calyx.search rendered hit freshness objects",
            "fresh_true_response": fresh,
            "fresh_false_response": stale_ok,
        }),
    );
}

#[test]
fn kernel_answer_ungrounded_fails_closed() {
    let _env = TestEnv::new("kernel-ungrounded");
    let server = server();
    vault_with_algorithmic_data(&server, "v");

    let error = call_err(
        &server,
        7,
        "calyx.kernel_answer",
        json!({"vault": "v", "query": "alpha"}),
    );

    assert_eq!(error.code, -32000);
    let data = error.data.unwrap();
    assert_eq!(data["calyx_code"], "CALYX_KERNEL_UNGROUNDED");
    assert_eq!(data["remediation"], "add anchors (grounding_gaps)");
}

#[test]
fn neighbors_returns_bounded_scores_for_known_cx() {
    let _env = TestEnv::new("neighbors");
    let server = server();
    let ingested = vault_with_algorithmic_data(&server, "v");
    let cx_id = ingested[0]["cx_id"].as_str().unwrap();

    let result = call_ok(
        &server,
        8,
        "calyx.neighbors",
        json!({"vault": "v", "cx_id": cx_id, "k": 5}),
    );

    let neighbors = result["neighbors"].as_array().unwrap();
    assert!(!neighbors.is_empty());
    assert!(neighbors.len() <= 5);
    for item in neighbors {
        let score = item["score"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&score));
        assert!(item["cx_id"].as_str().unwrap().len() == 32);
    }
}

#[test]
fn empty_vault_search_returns_empty_hits_without_error() {
    let _env = TestEnv::new("empty");
    let server = server();
    call_ok(&server, 9, "calyx.create_vault", json!({"name": "v"}));

    let result = call_ok(
        &server,
        10,
        "calyx.search",
        json!({"vault": "v", "query": "alpha"}),
    );

    assert_eq!(result["hits"].as_array().unwrap().len(), 0);
}

#[test]
fn invalid_search_arguments_are_invalid_params() {
    let _env = TestEnv::new("invalid");
    let server = server();

    let zero_k = call_err(
        &server,
        11,
        "calyx.search",
        json!({"vault": "v", "query": "alpha", "k": 0}),
    );
    let bad_fusion = call_err(
        &server,
        12,
        "calyx.search",
        json!({"vault": "v", "query": "alpha", "fusion": "unknown"}),
    );

    assert_eq!(zero_k.code, -32602);
    assert_eq!(bad_fusion.code, -32602);
}

#[test]
fn in_region_guard_requires_calibration() {
    let _env = TestEnv::new("guard");
    let server = server();
    vault_with_algorithmic_data(&server, "v");

    let error = call_err(
        &server,
        13,
        "calyx.search",
        json!({"vault": "v", "query": "alpha", "guard": "in_region"}),
    );

    assert_eq!(error.code, -32000);
    assert_eq!(error.data.unwrap()["calyx_code"], "CALYX_GUARD_PROVISIONAL");
}

#[test]
fn in_region_guard_uses_calibrated_ward_profile() {
    let env = TestEnv::new("guard-calibrated");
    let server = server();
    let created = call_ok(&server, 1, "calyx.create_vault", json!({"name": "v"}));
    call_ok(
        &server,
        2,
        "calyx.add_lens",
        json!({"vault": "v", "name": "byte_axis", "runtime": "algorithmic"}),
    );
    call_ok(
        &server,
        3,
        "calyx.ingest",
        json!({"vault": "v", "input": "alpha"}),
    );
    call_ok(
        &server,
        4,
        "calyx.ingest",
        json!({"vault": "v", "input": "beta"}),
    );
    let vault_id = created["vault_id"].as_str().unwrap();
    let vault_path = env.vault_path(vault_id);
    write_calibrated_default_guard(&vault_path, vault_id, "v", 0.0);

    let result = call_ok(
        &server,
        5,
        "calyx.search",
        json!({"vault": "v", "query": "alpha", "guard": "in_region", "explain": true}),
    );

    let hits = result["hits"].as_array().unwrap();
    assert!(!hits.is_empty());
    let evidence = &hits[0]["guard"]["evidence"];
    assert_eq!(evidence["mode"], "in_region_only");
    assert_eq!(evidence["verdict"]["overall_pass"], true);
    assert_eq!(evidence["verdict"]["provisional"], false);
    assert!(evidence["verdict"]["per_slot"].as_array().unwrap()[0]["tau"].is_number());
    assert!(result["dropped_guard_hits"].as_array().is_some());

    let state = load_vault_panel_state(&vault_path).expect("load panel state after search");
    let vault_id = vault_id.parse().expect("parse vault id");
    let vault = AsterVault::open(
        &vault_path,
        vault_id,
        crate::tools::vault::store::vault_salt(vault_id, "v"),
        VaultOptions::default(),
    )
    .expect("open vault readback");
    let guard_bytes = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Guard, b"profile\0default")
        .expect("read guard cf")
        .expect("guard row exists");
    let profile: GuardProfile = serde_json::from_slice(&guard_bytes).expect("profile readback");
    let slot_kinds = profile
        .calibration
        .as_ref()
        .map(|calibration| {
            calibration
                .per_slot
                .iter()
                .map(|(slot, meta)| {
                    json!({
                        "slot": slot.get(),
                        "slot_kind": meta.slot_kind.map(|kind| kind.label()),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    maybe_write_fsv_json(
        "mcp-guarded-search-readback.json",
        &json!({
            "source_of_truth": "Aster Guard CF profile\\0default row and JSON-RPC calyx.search response",
            "vault_path": vault_path,
            "panel_version": state.panel.version,
            "guard_cf": {
                "key_hex": "70726f66696c650064656661756c74",
                "bytes_len": guard_bytes.len(),
                "required_slots": profile.required_slots,
                "tau": profile.tau,
                "slot_kinds": slot_kinds,
                "calibrated": profile.is_calibrated(),
            },
            "search_response": result,
        }),
    );
}
