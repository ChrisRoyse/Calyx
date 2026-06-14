use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::{
    DedupAction, DedupPolicy, DedupResult, EpochSecs, IngestInput, TauStrategy, TctCosineConfig,
    ingest_at,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Modality, SlotId, SlotVector, VaultId};
use calyx_loom::recurrence::{PeriodicRecallQuery, SeriesStore};
use calyx_oracle::predict_next_occurrence;
use serde::Serialize;
use serde_json::{Value, json};

mod artifact;
mod parse;

use artifact::{ledger_payloads, raw_rows, vault_files, write_json};
use parse::{Args, LogEvent, assert_expected_cadence, cadence_gaps, parse_events};

const CALYX_TEMPORAL_LOG_EMPTY: &str = "CALYX_TEMPORAL_LOG_EMPTY";
const CALYX_TEMPORAL_LOG_CADENCE_MISMATCH: &str = "CALYX_TEMPORAL_LOG_CADENCE_MISMATCH";
const CALYX_TEMPORAL_LOG_SIGNATURE_MISMATCH: &str = "CALYX_TEMPORAL_LOG_SIGNATURE_MISMATCH";

pub(super) const CALYX_TEMPORAL_LOG_BAD_TIMESTAMP: &str = "CALYX_TEMPORAL_LOG_BAD_TIMESTAMP";
pub(super) const CALYX_TEMPORAL_LOG_NON_MONOTONIC: &str = "CALYX_TEMPORAL_LOG_NON_MONOTONIC";
pub(super) const CALYX_TEMPORAL_LOG_VAULT_NOT_EMPTY: &str = "CALYX_TEMPORAL_LOG_VAULT_NOT_EMPTY";

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &[u8] = b"issue610-temporal-real-log";
const PANEL_VERSION: u32 = 70;
const CONTENT_SLOT: u16 = 0;
const EVENT_BYTES: &[u8] = b"temp";

pub fn readback_temporal_log_recurrence(args: &[String]) -> crate::error::CliResult {
    let args = Args::parse(args)?;
    let bytes = std::fs::read(&args.log).map_err(|error| error.to_string())?;
    let text = String::from_utf8(bytes.clone()).map_err(|error| error.to_string())?;
    let events = parse_events(&text, args.rows)?;
    let gaps = cadence_gaps(&events)?;
    assert_expected_cadence(&gaps, args.expected_cadence_secs)?;
    ensure_empty_vault_target(&args.vault)?;

    let writer = AsterVault::new_durable(&args.vault, vault_id()?, SALT.to_vec(), vault_options()?)
        .map_err(|error| error.to_string())?;
    let (cx_id, ingest_results) = ingest_events(&writer, &events, &args)?;
    writer.flush().map_err(|error| error.to_string())?;
    drop(writer);

    let reader = AsterVault::open(&args.vault, vault_id()?, SALT.to_vec(), vault_options()?)
        .map_err(|error| error.to_string())?;
    let store = SeriesStore::new(&reader);
    let recurrence = store
        .recurrence_series(cx_id)
        .map_err(|error| error.to_string())?;
    let prediction = predict_next_occurrence(&reader, cx_id, args.confidence_ceiling)
        .map_err(|error| error.to_string())?;
    let recall = periodic_recall(&store, &recurrence)?;
    let ledger_payloads = ledger_payloads(&reader)?;

    let expected_next = events
        .last()
        .expect("events validated")
        .epoch_secs
        .checked_add(args.expected_cadence_secs)
        .ok_or_else(|| temporal_error(CALYX_TEMPORAL_LOG_CADENCE_MISMATCH, "next overflow"))?;
    let recurrence_signature_count = ledger_payloads
        .iter()
        .filter(|payload| payload["payload"]["recurrence_signature"] == json!(true))
        .count();
    validate_readback(
        expected_next,
        args.expected_cadence_secs,
        events.len(),
        recurrence_signature_count,
        &recurrence,
        &prediction,
    )?;

    let artifact = json!({
        "artifact_kind": "ph70.temporal-real-log-recurrence.v1",
        "source_of_truth": "real timestamped log file bytes plus cold-open Aster base/recurrence/online/ledger CF rows",
        "trigger": {
            "operation": "calyx readback temporal-log-recurrence",
            "event": "same content event ingested at each parsed real-log timestamp",
            "intended_outcome": "recurrence signature fires, recurrence series persists, periodic fit detects cadence, Oracle predicts next occurrence",
        },
        "input_log": {
            "path": args.log.display().to_string(),
            "bytes": bytes.len(),
            "blake3": blake3::hash(&bytes).to_string(),
            "selected_rows": events.len(),
            "sample": events,
        },
        "expected": {
            "cadence_secs": args.expected_cadence_secs,
            "gaps_secs": gaps,
            "next_occurrence_epoch_secs": expected_next,
            "dedup_results": "first row New, subsequent rows DedupMerge with recurrence_signature=true",
        },
        "actual": {
            "vault": args.vault.display().to_string(),
            "cx_id": cx_id.to_string(),
            "ingest_results": ingest_results,
            "recurrence": recurrence,
            "prediction": prediction,
            "periodic_recall": recall,
            "ledger_payloads": ledger_payloads,
            "raw_cf": {
                "base": raw_rows(&reader, ColumnFamily::Base)?,
                "recurrence": raw_rows(&reader, ColumnFamily::Recurrence)?,
                "online": raw_rows(&reader, ColumnFamily::Online)?,
                "ledger": raw_rows(&reader, ColumnFamily::Ledger)?,
            },
            "vault_files": vault_files(&args.vault)?,
        },
        "checks": {
            "cadence_matches_expected": true,
            "next_occurrence_matches_expected": true,
            "prediction_interval_contains_expected": true,
            "recurrence_signature_count": recurrence_signature_count,
            "recurrence_signature_expected": events.len().saturating_sub(1),
        },
    });
    write_json(&args.out, &artifact)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&artifact).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn ingest_events(
    vault: &AsterVault,
    events: &[LogEvent],
    args: &Args,
) -> Result<(calyx_core::CxId, Vec<Value>), String> {
    let mut cx_id = None;
    let mut results = Vec::new();
    for (index, event) in events.iter().enumerate() {
        let result = ingest_at(
            vault,
            &input_for_event(event, args),
            EpochSecs(event.epoch_secs),
            None,
        )
        .map_err(|error| error.to_string())?;
        match (&result, cx_id) {
            (DedupResult::New(id), None) if index == 0 => cx_id = Some(*id),
            (DedupResult::DedupMerge { into, occurrence }, Some(expected))
                if *into == expected && occurrence.0 == index as u64 => {}
            _ => {
                return Err(temporal_error(
                    CALYX_TEMPORAL_LOG_SIGNATURE_MISMATCH,
                    format!("unexpected dedup result at selected row {index}: {result:?}"),
                ));
            }
        }
        results.push(json!({
            "line_number": event.line_number,
            "timestamp": event.timestamp,
            "epoch_secs": event.epoch_secs,
            "result": result,
        }));
    }
    cx_id
        .ok_or_else(|| temporal_error(CALYX_TEMPORAL_LOG_EMPTY, "no events ingested"))
        .map(|id| (id, results))
}

fn input_for_event(_event: &LogEvent, _args: &Args) -> IngestInput {
    IngestInput::new(EVENT_BYTES.to_vec(), PANEL_VERSION, Modality::Text).with_slot(
        SlotId::new(CONTENT_SLOT),
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0, 0.0],
        },
    )
}

fn periodic_recall(
    store: &SeriesStore<'_, calyx_core::SystemClock>,
    recurrence: &impl Serialize,
) -> Result<Value, String> {
    let value = serde_json::to_value(recurrence).map_err(|error| error.to_string())?;
    let fit = &value["periodic_fit"];
    let hour = fit["target_hour"].as_u64().map(|value| value as u8);
    let day = fit["target_day_of_week"].as_u64().map(|value| value as u8);
    let query = PeriodicRecallQuery::new(hour, day).map_err(|error| error.to_string())?;
    store
        .periodic_recall_readback(query)
        .map(|readback| serde_json::to_value(readback).expect("periodic recall json"))
        .map_err(|error| error.to_string())
}

fn validate_readback(
    expected_next: i64,
    expected_cadence: i64,
    selected_rows: usize,
    signature_count: usize,
    recurrence: &calyx_loom::recurrence::RecurrenceRead,
    prediction: &calyx_oracle::TimePrediction,
) -> Result<(), String> {
    if recurrence.series.occurrences.len() != selected_rows {
        return Err(temporal_error(
            CALYX_TEMPORAL_LOG_SIGNATURE_MISMATCH,
            "persisted occurrence count does not match selected log rows",
        ));
    }
    if recurrence.series.cadence_secs != Some(expected_cadence as f64)
        || recurrence.periodic_fit.dominant_period_secs != Some(expected_cadence as f64)
    {
        return Err(temporal_error(
            CALYX_TEMPORAL_LOG_CADENCE_MISMATCH,
            "persisted cadence or periodic fit does not match expected cadence",
        ));
    }
    if prediction.t_hat.0 != expected_next
        || prediction.interval.low.0 > expected_next
        || prediction.interval.high.0 < expected_next
    {
        return Err(temporal_error(
            CALYX_TEMPORAL_LOG_CADENCE_MISMATCH,
            "Oracle prediction does not match expected next timestamp",
        ));
    }
    if signature_count != selected_rows.saturating_sub(1) {
        return Err(temporal_error(
            CALYX_TEMPORAL_LOG_SIGNATURE_MISMATCH,
            "ledger recurrence_signature count does not match merge count",
        ));
    }
    Ok(())
}

fn ensure_empty_vault_target(vault: &Path) -> std::result::Result<(), String> {
    if !vault.exists() {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(vault).map_err(|error| error.to_string())?;
    if entries.next().is_none() {
        return Ok(());
    }
    Err(temporal_error(
        CALYX_TEMPORAL_LOG_VAULT_NOT_EMPTY,
        format!("vault target is not empty: {}", vault.display()),
    ))
}

fn vault_options() -> Result<VaultOptions, String> {
    Ok(VaultOptions {
        dedup_policy: Some(DedupPolicy::TctCosine(
            TctCosineConfig::new(
                vec![SlotId::new(CONTENT_SLOT)],
                TauStrategy::PerSlot(vec![(SlotId::new(CONTENT_SLOT), 0.90)]),
                DedupAction::RecurrenceSeries,
            )
            .map_err(|error| error.to_string())?,
        )),
        ..VaultOptions::default()
    })
}

fn vault_id() -> Result<VaultId, String> {
    VAULT_ID
        .parse()
        .map_err(|error| format!("vault id parse: {error}"))
}

fn temporal_error(code: &'static str, message: impl Into<String>) -> String {
    format!("{code}: {}", message.into())
}
