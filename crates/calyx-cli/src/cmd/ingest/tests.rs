use std::collections::BTreeMap;
use std::fs;

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AbsentReason, Anchor, AnchorKind, AnchorValue, Asymmetry, CxId, Input, LensId, Modality, Panel,
    QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, SlotVector, VaultId, VaultStore,
};
use calyx_registry::{Registry, load_vault_panel_state, persist_vault_panel_state};
use proptest::prelude::*;
use serde_json::json;
use ulid::Ulid;

use super::super::vault::{ResolvedVault, now_ms, vault_salt};
use super::anchor::parse_anchor_kind;
use super::batch::{parse_batch_line, read_batch_texts, validate_batch_file};
use super::command::{
    ingest_batch_streaming, ingest_batch_streaming_with_summary_emitter, ingest_texts,
    ingest_validated_batch_streaming_with_output, should_stage_batch_constellation,
};
use super::constellation::{measure_constellation, measure_constellation_microbatch, text_input};
use super::parse::{parse_anchor, validate_text};
use super::store::{ensure_base_exists, open_vault};
use super::types::BatchIngestSummary;

mod support;
use support::*;

mod anchor_replay;
mod basic;
mod batch_edges;
mod rebuild_marker;
mod retention;
mod route_gate;
mod session_status;

fn ingest_cf_state(resolved: &ResolvedVault) -> serde_json::Value {
    let vault = open_vault(resolved).unwrap();
    let snapshot = vault.snapshot();
    json!({
        "latest_seq": snapshot,
        "base_rows": vault.scan_cf_at(snapshot, ColumnFamily::Base).unwrap().len(),
        "anchors_rows": vault.scan_cf_at(snapshot, ColumnFamily::Anchors).unwrap().len(),
        "ledger_rows": vault.scan_cf_at(snapshot, ColumnFamily::Ledger).unwrap().len(),
        "slot_00_rows": vault
            .scan_cf_at(snapshot, ColumnFamily::slot(SlotId::new(0)))
            .unwrap()
            .len(),
        "cf_files": {
            "base": cf_file_count(&resolved.path, ColumnFamily::Base),
            "anchors": cf_file_count(&resolved.path, ColumnFamily::Anchors),
            "ledger": cf_file_count(&resolved.path, ColumnFamily::Ledger),
            "slot_00": cf_file_count(&resolved.path, ColumnFamily::slot(SlotId::new(0))),
        },
    })
}

fn write_issue911_fsv(
    resolved: &ResolvedVault,
    before: &serde_json::Value,
    after: &serde_json::Value,
    error_code: &str,
    error_message: &str,
) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    let root = root.join("issue911-cli-ingest-unavailable");
    fs::create_dir_all(&root).unwrap();
    let artifact = json!({
        "issue": 911,
        "source_of_truth": "Aster durable CF scans at vault snapshot: Base, Ledger, slot_00",
        "trigger": "CLI ingest text into a panel whose only applicable text lens is unregistered",
        "expected": {
            "error_code": "CALYX_LENS_UNREACHABLE",
            "base_rows_after": 0,
            "ledger_rows_after": 0,
            "slot_00_rows_after": 0,
        },
        "observed_error": {
            "code": error_code,
            "message": error_message,
        },
        "before": before,
        "after": after,
        "physical_cf_dirs_exist": {
            "base": resolved.path.join("cf").join("base").is_dir(),
            "ledger": resolved.path.join("cf").join("ledger").is_dir(),
            "slot_00": resolved.path.join("cf").join("slot_00").is_dir(),
        },
    });
    fs::write(
        root.join("cli-ingest-unavailable-fail-closed.json"),
        serde_json::to_vec_pretty(&artifact).unwrap(),
    )
    .unwrap();
}

fn cf_file_count(root: &std::path::Path, cf: ColumnFamily) -> usize {
    let dir = root.join("cf").join(cf.name());
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_file())
                .count()
        })
        .unwrap_or(0)
}
