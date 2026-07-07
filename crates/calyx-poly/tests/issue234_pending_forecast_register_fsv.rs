//! Issue #234 - pending forecast register and resolution join.
//!
//! Source of truth: durable AsterVault Ledger CF rows plus persisted register/readback JSON.

#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::Path;

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultStore;
use calyx_ledger::{EntryKind, decode as decode_ledger};
use calyx_poly::{ERR_PENDING_FORECAST_INVALID, Resolution};
use calyx_poly::{
    ForecastSource, PendingForecastEntry, PendingForecastRegister, PendingForecastStatus,
    join_resolution_to_pending_forecasts, observe_pending_forecasts, record_pending_forecast,
};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue234-pending-forecast-register";
const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const HASH_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const HASH_D: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

#[test]
fn issue234_pending_forecast_register_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE234_FSV_ROOT",
        "issue234-pending-forecast-register",
    );
    reset_dir(&root);

    let happy = happy_scored_roundtrip(&root);
    let multi = multiple_versions_idempotent_no_lookahead(&root);
    let voided = voided_resolution_marks_void(&root);
    let no_match = no_match_and_never_resolved_observability(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 234,
        "proof_claim": "Pending forecasts are durably registered, joined to causal resolutions by condition_id/outcome mapping, transitioned exactly once to Scored or Void, and left observable when unresolved.",
        "minimum_sufficient_proof_corpus": {
            "cases": 4,
            "why_this_is_sufficient": "One happy scored transition proves the base register/join; one multi-version case also proves all-match selection, idempotent replay, and no-lookahead blocking; one voided case proves Void without scoring; one no-match case also proves a never-resolved forecast remains Pending and observable.",
            "why_larger_is_wasteful": "More forecasts would repeat the same join states without adding a new #234 behavior."
        },
        "source_of_truth": "durable AsterVault Ledger CF rows plus persisted register/readback JSON",
        "cases": {
            "happy": happy,
            "multi_idempotent_no_lookahead": multi,
            "voided": voided,
            "no_match_never_resolved": no_match
        },
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue234_pending_forecast_register_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

fn happy_scored_roundtrip(root: &Path) -> Value {
    let (vault, case_root) = case_vault(root, "happy");
    let mut register = PendingForecastRegister::default();
    let reg_ref = record_pending_forecast(
        &vault,
        &mut register,
        forecast("f234happy", "cond234happy", 0, 100, 1, "short", HASH_A),
    )
    .expect("record happy pending");
    vault.flush().expect("flush registered");
    let registered = ledger_payload(&vault, reg_ref.seq);
    assert_eq!(
        registered["event"],
        json!("poly.pending_forecast_registered")
    );
    assert_eq!(registered["forecast"]["status"], json!("pending"));

    let result = join_resolution_to_pending_forecasts(
        &vault,
        &mut register,
        &resolution("cond234happy", 0, 200, "uma-onchain"),
        false,
    )
    .expect("join happy");
    vault.flush().expect("flush happy join");
    assert_eq!(result.selected_forecast_ids, vec!["f234happy"]);
    assert_eq!(result.transitioned_forecast_ids, vec!["f234happy"]);
    assert_eq!(register.entries[0].status, PendingForecastStatus::Scored);
    assert_eq!(result.work_items[0].actual_win, Some(true));
    let joined = ledger_payload(&vault, result.ledger_seq.expect("join ledger"));
    assert_eq!(joined["work_items"][0]["actual_win"], json!(true));
    persist_case(
        &case_root,
        json!({
            "registered": registered,
            "join_result": result,
            "join_ledger": joined,
            "register": register
        }),
    )
}

fn multiple_versions_idempotent_no_lookahead(root: &Path) -> Value {
    let (vault, case_root) = case_vault(root, "multi-idempotent-no-lookahead");
    let mut register = PendingForecastRegister::default();
    for entry in [
        forecast("f234m1", "cond234multi", 0, 100, 1, "1h", HASH_A),
        forecast("f234m2", "cond234multi", 1, 120, 2, "6h", HASH_B),
        forecast("f234m3", "cond234multi", 0, 130, 3, "24h", HASH_C),
        forecast("f234future", "cond234multi", 0, 250, 4, "24h", HASH_D),
    ] {
        record_pending_forecast(&vault, &mut register, entry).expect("record multi pending");
    }
    vault.flush().expect("flush multi registered");
    let resolution = resolution("cond234multi", 0, 200, "uma-onchain");
    let first = join_resolution_to_pending_forecasts(&vault, &mut register, &resolution, false)
        .expect("first multi join");
    vault.flush().expect("flush multi join");
    assert_eq!(
        first.selected_forecast_ids,
        vec!["f234m1", "f234m2", "f234m3"]
    );
    assert_eq!(first.lookahead_blocked_forecast_ids, vec!["f234future"]);
    assert_eq!(
        observe_pending_forecasts(&register, 300, 1).pending_forecast_ids,
        vec!["f234future"]
    );
    let replay = join_resolution_to_pending_forecasts(&vault, &mut register, &resolution, false)
        .expect("replay multi join");
    assert!(replay.idempotent_replay);
    assert_eq!(replay.ledger_seq, None);
    assert_eq!(replay.transitioned_forecast_ids, Vec::<String>::new());
    assert_eq!(replay.selected_forecast_ids, first.selected_forecast_ids);
    let joined = ledger_payload(&vault, first.ledger_seq.expect("multi join ledger"));
    persist_case(
        &case_root,
        json!({
            "first_join": first,
            "replay_join": replay,
            "join_ledger": joined,
            "register": register
        }),
    )
}

fn voided_resolution_marks_void(root: &Path) -> Value {
    let (vault, case_root) = case_vault(root, "voided");
    let mut register = PendingForecastRegister::default();
    record_pending_forecast(
        &vault,
        &mut register,
        forecast("f234voidyes", "cond234void", 0, 100, 1, "1h", HASH_A),
    )
    .expect("record void yes");
    record_pending_forecast(
        &vault,
        &mut register,
        forecast("f234voidno", "cond234void", 1, 100, 1, "1h", HASH_B),
    )
    .expect("record void no");
    let result = join_resolution_to_pending_forecasts(
        &vault,
        &mut register,
        &resolution("cond234void", 0, 200, "uma-invalid"),
        true,
    )
    .expect("void join");
    vault.flush().expect("flush void");
    assert_eq!(
        result.selected_forecast_ids,
        vec!["f234voidyes", "f234voidno"]
    );
    assert!(
        result
            .work_items
            .iter()
            .all(|item| item.actual_win.is_none())
    );
    assert!(
        register
            .entries
            .iter()
            .all(|entry| entry.status == PendingForecastStatus::Void)
    );
    let joined = ledger_payload(&vault, result.ledger_seq.expect("void ledger"));
    persist_case(
        &case_root,
        json!({
            "join_result": result,
            "join_ledger": joined,
            "register": register
        }),
    )
}

fn no_match_and_never_resolved_observability(root: &Path) -> Value {
    let (vault, case_root) = case_vault(root, "no-match-never-resolved");
    let mut register = PendingForecastRegister::default();
    record_pending_forecast(
        &vault,
        &mut register,
        forecast("f234never", "cond234never", 0, 100, 1, "1h", HASH_A),
    )
    .expect("record never-resolved forecast");
    let result = join_resolution_to_pending_forecasts(
        &vault,
        &mut register,
        &resolution("cond234noentry", 0, 200, "uma-onchain"),
        false,
    )
    .expect("no-match join");
    vault.flush().expect("flush no-match");
    assert!(result.selected_forecast_ids.is_empty());
    assert!(result.transitioned_forecast_ids.is_empty());
    assert_eq!(result.pending_after, 1);
    assert_eq!(register.entries[0].status, PendingForecastStatus::Pending);
    let obs = observe_pending_forecasts(&register, 10_000, 60);
    assert_eq!(obs.pending_forecast_ids, vec!["f234never"]);
    assert_eq!(obs.stale_pending_forecast_ids, vec!["f234never"]);
    let joined = ledger_payload(&vault, result.ledger_seq.expect("no-match ledger"));
    persist_case(
        &case_root,
        json!({
            "join_result": result,
            "join_ledger": joined,
            "observability": obs,
            "register": register
        }),
    )
}

fn case_vault(root: &Path, name: &str) -> (AsterVault, std::path::PathBuf) {
    let case_root = root.join(name);
    reset_dir(&case_root);
    let vault = AsterVault::new_durable(
        case_root.join("vault"),
        VAULT_ID.parse().unwrap(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open vault");
    (vault, case_root)
}

fn forecast(
    forecast_id: &str,
    condition_id: &str,
    outcome_index: u32,
    forecast_ts: u64,
    version: u32,
    horizon: &str,
    hash: &str,
) -> PendingForecastEntry {
    PendingForecastEntry {
        forecast_id: forecast_id.to_string(),
        source: ForecastSource::CalyxNative,
        condition_id: condition_id.to_string(),
        token_id: format!("{condition_id}-token-{outcome_index}"),
        outcome_index,
        domain: "crypto".to_string(),
        horizon_bucket: horizon.to_string(),
        forecast_version: version,
        p_model: if outcome_index == 0 { 0.72 } else { 0.28 },
        confidence: 0.66,
        forecast_ts,
        provenance_hash: hash.to_string(),
        status: PendingForecastStatus::Pending,
        registered_ledger_seq: None,
        terminal_ledger_seq: None,
        terminal_resolution_id: None,
        terminal_actual_win: None,
    }
}

fn resolution(condition_id: &str, winning: u32, resolved_ts: u64, source: &str) -> Resolution {
    Resolution {
        condition_id: condition_id.to_string(),
        winning_outcome_index: winning,
        winning_label: if winning == 0 { "YES" } else { "NO" }.to_string(),
        resolved_ts,
        source: source.to_string(),
        disputed: false,
    }
}

fn ledger_payload(vault: &AsterVault, seq: u64) -> Value {
    let row = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(seq))
        .expect("read ledger")
        .expect("ledger present");
    let ledger = decode_ledger(&row).expect("decode ledger");
    assert_eq!(ledger.kind, EntryKind::Measure);
    serde_json::from_slice(&ledger.payload).expect("decode payload")
}

fn persist_case(case_root: &Path, value: Value) -> Value {
    let path = case_root.join("readback.json");
    write_json(&path, &value);
    let readback: Value =
        serde_json::from_slice(&fs::read(&path).expect("read case")).expect("decode case");
    assert_eq!(readback, value);
    json!({
        "path": path.display().to_string(),
        "readback_equal": true,
        "value": readback
    })
}

#[test]
fn pending_forecast_rejects_bad_hash() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE234_INVALID_ROOT", "issue234-pending-invalid");
    reset_dir(&root);
    let (vault, _) = case_vault(&root, "invalid");
    let mut register = PendingForecastRegister::default();
    let err = record_pending_forecast(
        &vault,
        &mut register,
        forecast("f234bad", "cond234bad", 0, 100, 1, "1h", "bad"),
    )
    .expect_err("bad provenance hash must fail closed");
    assert_eq!(err.code(), ERR_PENDING_FORECAST_INVALID);
}
