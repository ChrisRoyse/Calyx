//! Issue #240 - replay pending forecast register from Aster Ledger CF.
//!
//! Source of truth: a reopened durable AsterVault Ledger CF plus persisted readback JSON.

#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode as decode_ledger};
use calyx_poly::pending_forecast_replay::{
    ERR_PENDING_FORECAST_REPLAY_DUPLICATE, ERR_PENDING_FORECAST_REPLAY_REF,
    replay_pending_forecast_register_from_vault,
};
use calyx_poly::{
    ForecastSource, PendingForecastEntry, PendingForecastRegister, PendingForecastStatus,
    Resolution, join_resolution_to_pending_forecasts, observe_pending_forecasts,
    record_pending_forecast,
};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue240-pending-forecast-replay";
const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const HASH_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const HASH_D: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const HASH_E: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

#[test]
fn issue240_pending_forecast_replay_fsv() {
    let (root, _keep) =
        named_fsv_root("POLY_ISSUE240_FSV_ROOT", "issue240-pending-forecast-replay");
    #[cfg(windows)]
    assert!(
        root.to_string_lossy().starts_with("C:"),
        "issue240 FSV root must stay on C:"
    );
    reset_dir(&root);

    let replay = reopened_vault_reconstructs_register(&root);
    let edges = edge_cases_fail_closed(&root);
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 240,
        "proof_claim": "Pending forecast entries and terminal joins are reconstructed from reopened Aster Ledger CF rows after safe-ref payload migration, with long IDs restored exactly and no-look-ahead state preserved.",
        "minimum_sufficient_proof_corpus": {
            "cases": 4,
            "rows": "one still-pending long-ID forecast, one two-entry scored resolution join, one voided forecast, and one look-ahead-blocked future forecast",
            "why_this_is_sufficient": "These rows cover every durable state transition: Registered->Pending, Registered->Scored, Registered->Void, and causal non-transition for future forecasts. Additional market history would only repeat equivalent states.",
            "why_larger_is_wasteful": "No statistical estimate is being validated; replay correctness is structural over ledger event types and safe-ref reconstruction."
        },
        "source_of_truth": "reopened durable AsterVault Ledger CF rows plus persisted JSON readback",
        "replay": replay,
        "edge_cases": edges,
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue240_pending_forecast_replay_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

fn reopened_vault_reconstructs_register(root: &Path) -> Value {
    let case_root = root.join("reopened-replay");
    reset_dir(&case_root);
    let vault_dir = case_root.join("vault");
    let multi_resolution = resolution(long_condition("multi"), 0, 200, "uma-onchain");
    let void_resolution = resolution(long_condition("void"), 1, 210, "uma-invalid");
    let pending = forecast(
        "pending",
        long_condition("pending"),
        long_token("pending", 0),
        100,
        1,
        "1h",
        HASH_A,
    );
    let scored_a = forecast(
        "multi-a",
        multi_resolution.condition_id.clone(),
        long_token("multi", 0),
        100,
        1,
        "1h",
        HASH_B,
    );
    let scored_b = forecast(
        "multi-b",
        multi_resolution.condition_id.clone(),
        long_token("multi", 1),
        110,
        2,
        "6h",
        HASH_C,
    );
    let future = forecast(
        "future-blocked",
        multi_resolution.condition_id.clone(),
        long_token("multi", 2),
        250,
        3,
        "24h",
        HASH_D,
    );
    let voided = forecast(
        "voided",
        void_resolution.condition_id.clone(),
        long_token("void", 1),
        120,
        1,
        "1h",
        HASH_E,
    );

    {
        let vault = open_vault(&vault_dir);
        let mut register = PendingForecastRegister::default();
        for entry in [
            pending.clone(),
            scored_a.clone(),
            scored_b.clone(),
            future.clone(),
            voided.clone(),
        ] {
            record_pending_forecast(&vault, &mut register, entry).expect("record pending forecast");
        }
        join_resolution_to_pending_forecasts(&vault, &mut register, &multi_resolution, false)
            .expect("join scored forecasts");
        join_resolution_to_pending_forecasts(&vault, &mut register, &void_resolution, true)
            .expect("join voided forecast");
        vault.flush().expect("flush source ledger");
    }

    let reopened = open_vault(&vault_dir);
    let before_seq = reopened.latest_seq();
    let mut replayed =
        replay_pending_forecast_register_from_vault(&reopened).expect("replay register");
    assert_eq!(replayed.entries.len(), 5);
    assert_entry(
        &replayed,
        &pending.forecast_id,
        PendingForecastStatus::Pending,
        &pending,
    );
    assert_entry(
        &replayed,
        &scored_a.forecast_id,
        PendingForecastStatus::Scored,
        &scored_a,
    );
    assert_entry(
        &replayed,
        &scored_b.forecast_id,
        PendingForecastStatus::Scored,
        &scored_b,
    );
    assert_entry(
        &replayed,
        &future.forecast_id,
        PendingForecastStatus::Pending,
        &future,
    );
    assert_entry(
        &replayed,
        &voided.forecast_id,
        PendingForecastStatus::Void,
        &voided,
    );

    let replay_join =
        join_resolution_to_pending_forecasts(&reopened, &mut replayed, &multi_resolution, false)
            .expect("idempotent join after replay");
    assert!(replay_join.idempotent_replay);
    assert_eq!(replay_join.ledger_seq, None);
    assert_eq!(reopened.latest_seq(), before_seq);
    assert_eq!(
        replay_join.lookahead_blocked_forecast_ids,
        vec![future.forecast_id.clone()]
    );
    let obs = observe_pending_forecasts(&replayed, 1_000, 1);
    assert_eq!(obs.pending_count, 2);
    assert!(obs.pending_forecast_ids.contains(&pending.forecast_id));
    assert!(obs.pending_forecast_ids.contains(&future.forecast_id));

    let ledger_rows = ledger_payloads(&reopened);
    let value = json!({
        "vault_dir": vault_dir.display().to_string(),
        "ledger_row_count": ledger_rows.len(),
        "ledger_rows": ledger_rows,
        "replayed_register": replayed,
        "idempotent_join_after_replay": replay_join,
        "observability_after_replay": obs
    });
    persist_case(&case_root, "replay-readback.json", value)
}

fn edge_cases_fail_closed(root: &Path) -> Value {
    let hash = corrupt_ref_case(root, "corrupt-hash", |payload| {
        *payload
            .pointer_mut("/forecast/condition_ref/ref_hash")
            .expect("condition ref hash") = json!("bad-hash");
    });
    assert_eq!(
        hash["value"]["error_code"],
        json!(ERR_PENDING_FORECAST_REPLAY_REF)
    );

    let byte_len = corrupt_ref_case(root, "corrupt-byte-len", |payload| {
        *payload
            .pointer_mut("/forecast/outcome_ref/byte_len")
            .expect("outcome ref len") = json!(9999);
    });
    assert_eq!(
        byte_len["value"]["error_code"],
        json!(ERR_PENDING_FORECAST_REPLAY_REF)
    );

    let duplicate = duplicate_register_case(root);
    assert_eq!(
        duplicate["value"]["error_code"],
        json!(ERR_PENDING_FORECAST_REPLAY_DUPLICATE)
    );
    json!({
        "corrupt_ref_hash": hash,
        "corrupt_ref_byte_len": byte_len,
        "duplicate_registration": duplicate
    })
}

fn corrupt_ref_case<F>(root: &Path, name: &str, mutate: F) -> Value
where
    F: FnOnce(&mut Value),
{
    let case_root = root.join(name);
    reset_dir(&case_root);
    let vault_dir = case_root.join("vault");
    {
        let vault = open_vault(&vault_dir);
        let entry = forecast(
            "edge",
            long_condition(name),
            long_token(name, 0),
            100,
            1,
            "1h",
            HASH_A,
        );
        let mut payload = registered_payload(&entry);
        mutate(&mut payload);
        append_payload(&vault, payload);
        vault.flush().expect("flush corrupt edge");
    }
    let reopened = open_vault(&vault_dir);
    let err = replay_pending_forecast_register_from_vault(&reopened)
        .expect_err("corrupt safe-ref must fail closed");
    persist_case(
        &case_root,
        "edge-readback.json",
        json!({
            "error_code": err.code(),
            "error_kind": err.kind(),
            "error_message": err.message()
        }),
    )
}

fn duplicate_register_case(root: &Path) -> Value {
    let case_root = root.join("duplicate-register");
    reset_dir(&case_root);
    let vault_dir = case_root.join("vault");
    {
        let vault = open_vault(&vault_dir);
        let mut register = PendingForecastRegister::default();
        let entry = forecast(
            "duplicate",
            long_condition("duplicate"),
            long_token("duplicate", 0),
            100,
            1,
            "1h",
            HASH_A,
        );
        record_pending_forecast(&vault, &mut register, entry.clone()).expect("record first");
        record_pending_forecast(&vault, &mut register, entry).expect("record duplicate");
        vault.flush().expect("flush duplicate");
    }
    let reopened = open_vault(&vault_dir);
    let err = replay_pending_forecast_register_from_vault(&reopened)
        .expect_err("duplicate registration must fail closed");
    persist_case(
        &case_root,
        "edge-readback.json",
        json!({
            "error_code": err.code(),
            "error_kind": err.kind(),
            "error_message": err.message()
        }),
    )
}

fn forecast(
    suffix: &str,
    condition_id: String,
    token_id: String,
    forecast_ts: u64,
    version: u32,
    horizon: &str,
    hash: &str,
) -> PendingForecastEntry {
    PendingForecastEntry {
        forecast_id: format!("forecast-issue240-{suffix}-{}", "f".repeat(64)),
        source: ForecastSource::CalyxNative,
        condition_id,
        token_id,
        outcome_index: version.saturating_sub(1),
        domain: "crypto".to_string(),
        horizon_bucket: horizon.to_string(),
        forecast_version: version,
        p_model: 0.61,
        confidence: 0.51,
        forecast_ts,
        provenance_hash: hash.to_string(),
        status: PendingForecastStatus::Pending,
        registered_ledger_seq: None,
        terminal_ledger_seq: None,
        terminal_resolution_id: None,
        terminal_actual_win: None,
    }
}

fn resolution(condition_id: String, winning: u32, resolved_ts: u64, source: &str) -> Resolution {
    Resolution {
        condition_id,
        winning_outcome_index: winning,
        winning_label: if winning == 0 { "YES" } else { "NO" }.to_string(),
        resolved_ts,
        source: source.to_string(),
        disputed: false,
    }
}

fn assert_entry(
    register: &PendingForecastRegister,
    forecast_id: &str,
    status: PendingForecastStatus,
    expected: &PendingForecastEntry,
) {
    let entry = register
        .entries
        .iter()
        .find(|entry| entry.forecast_id == forecast_id)
        .expect("entry present");
    assert_eq!(entry.status, status);
    assert_eq!(entry.forecast_id, expected.forecast_id);
    assert_eq!(entry.condition_id, expected.condition_id);
    assert_eq!(entry.token_id, expected.token_id);
    assert!(entry.registered_ledger_seq.is_some());
}

fn registered_payload(entry: &PendingForecastEntry) -> Value {
    json!({
        "schema_version": "poly.pending_forecast_register.v1",
        "event": "poly.pending_forecast_registered",
        "forecast": {
            "forecast_ref": safe_ref_value(&entry.forecast_id),
            "source": entry.source,
            "condition_ref": safe_ref_value(&entry.condition_id),
            "outcome_ref": safe_ref_value(&entry.token_id),
            "outcome_index": entry.outcome_index,
            "domain": entry.domain,
            "horizon_bucket": entry.horizon_bucket,
            "forecast_version": entry.forecast_version,
            "p_model": entry.p_model,
            "confidence": entry.confidence,
            "forecast_ts": entry.forecast_ts,
            "provenance_hash": entry.provenance_hash,
            "status": entry.status,
            "registered_ledger_seq": null,
            "terminal_ledger_seq": null,
            "terminal_resolution_ref": null,
            "terminal_actual_win": null
        }
    })
}

fn safe_ref_value(value: &str) -> Value {
    json!({
        "ref_hash": blake3::hash(value.as_bytes()).to_hex().to_string(),
        "byte_len": value.len(),
        "chunks": value.as_bytes().chunks(12).map(|chunk| String::from_utf8_lossy(chunk).to_string()).collect::<Vec<_>>()
    })
}

fn append_payload(vault: &AsterVault, payload: Value) {
    vault
        .append_ledger_entry(
            EntryKind::Measure,
            SubjectId::Query(blake3::hash(b"issue240-edge").as_bytes().to_vec()),
            serde_json::to_vec(&payload).expect("encode payload"),
            ActorId::Service("issue240-fsv".to_string()),
        )
        .expect("append manual ledger payload");
}

fn ledger_payloads(vault: &AsterVault) -> Vec<Value> {
    let mut rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .expect("scan ledger");
    rows.sort_by_key(|(key, _)| u64::from_be_bytes(key.as_slice().try_into().expect("seq key")));
    rows.into_iter()
        .map(|(key, bytes)| {
            let seq = u64::from_be_bytes(key.as_slice().try_into().expect("seq key"));
            let by_point = vault
                .read_cf_at(vault.latest_seq(), ColumnFamily::Ledger, &ledger_key(seq))
                .expect("point read ledger")
                .expect("ledger row");
            assert_eq!(by_point, bytes);
            let ledger = decode_ledger(&bytes).expect("decode ledger");
            json!({
                "seq": seq,
                "kind": format!("{:?}", ledger.kind),
                "payload": serde_json::from_slice::<Value>(&ledger.payload).expect("decode payload")
            })
        })
        .collect()
}

fn persist_case(case_root: &Path, file: &str, value: Value) -> Value {
    let path = case_root.join(file);
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

fn open_vault(vault_dir: &PathBuf) -> AsterVault {
    AsterVault::open(
        vault_dir,
        VAULT_ID.parse().expect("vault id"),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open issue240 vault")
}

fn long_condition(name: &str) -> String {
    format!("0x{:0<64}", format!("240{name}"))
}

fn long_token(name: &str, index: u32) -> String {
    format!("token-issue240-{name}-{index}-{}", "1234567890".repeat(8))
}
