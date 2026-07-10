//! Scheduled, resumable pre-resolution crypto capture harness (issue #238).
//!
//! The harness composes the #38 crypto ingestor and #234 pending-forecast register. Its durable
//! state file is the scheduler source of truth: duplicate interval captures are refused before any
//! source fetch runs, pending entries can be hydrated after restart, and matured pre-resolution
//! pairs are emitted only after the pending register accepts a resolution join.

use std::path::{Path, PathBuf};

use calyx_core::{SystemClock, VaultId, VaultStore};

pub use crate::crypto_capture_harness_types::{
    CRYPTO_CAPTURE_HARNESS_SCHEMA_VERSION, CRYPTO_CAPTURE_REPORT_FILE, CRYPTO_CAPTURE_STATE_FILE,
    CRYPTO_PRE_RESOLUTION_CORPUS_FILE, CryptoCaptureDecisionKind, CryptoCaptureHarnessConfig,
    CryptoCaptureHarnessReport, CryptoCaptureHarnessRequest, CryptoCaptureHarnessRun,
    CryptoCaptureHarnessState, CryptoCaptureRecord, CryptoCaptureResolutionRun,
    CryptoCapturedSnapshotRef, CryptoMaturedResolutionRecord, CryptoPreResolutionPair,
    ERR_CRYPTO_CAPTURE_INVALID_CONFIG, ERR_CRYPTO_CAPTURE_LOOKAHEAD,
    ERR_CRYPTO_CAPTURE_NO_MATURED_PAIR, ERR_CRYPTO_CAPTURE_PENDING_ENTRY,
    ERR_CRYPTO_CAPTURE_READBACK,
};
use crate::crypto_ingestor::{
    CryptoIngestionRun, CryptoIngestorConfig, reject_forbidden_drive,
    run_live_crypto_ingestion_cycle,
};
use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};
use crate::live_calyx_native_evidence::LiveCalyxNativeEvidenceStore;
use crate::model::Resolution;
use crate::pending_forecast_register::{
    PendingForecastLedgerStore, PendingForecastRegister, PendingForecastWorkItem,
    ResolutionJoinResult, join_resolution_to_pending_forecasts,
};

pub trait CryptoCaptureRunner<S>
where
    S: VaultStore + PendingForecastLedgerStore,
{
    fn run_capture_cycle(
        &mut self,
        store: &S,
        register: &mut PendingForecastRegister,
        vault_id: VaultId,
        vault_salt: &[u8],
        output_root: &Path,
        config: CryptoIngestorConfig,
    ) -> Result<CryptoIngestionRun>;
}

#[derive(Default)]
pub struct LiveCryptoCaptureRunner;

impl<S> CryptoCaptureRunner<S> for LiveCryptoCaptureRunner
where
    S: VaultStore + PendingForecastLedgerStore + LiveCalyxNativeEvidenceStore,
{
    fn run_capture_cycle(
        &mut self,
        store: &S,
        register: &mut PendingForecastRegister,
        vault_id: VaultId,
        vault_salt: &[u8],
        output_root: &Path,
        config: CryptoIngestorConfig,
    ) -> Result<CryptoIngestionRun> {
        Ok(run_live_crypto_ingestion_cycle(
            store,
            register,
            vault_id,
            vault_salt,
            output_root,
            config,
            &SystemClock,
        )?
        .run)
    }
}

pub fn run_crypto_capture_harness_once<S, R>(
    store: &S,
    register: &mut PendingForecastRegister,
    request: CryptoCaptureHarnessRequest<'_>,
    runner: &mut R,
) -> Result<CryptoCaptureHarnessRun>
where
    S: VaultStore + PendingForecastLedgerStore,
    R: CryptoCaptureRunner<S>,
{
    let output_root = request.output_root;
    let config = request.config;
    reject_forbidden_drive(output_root)?;
    validate_config(&config)?;
    let state_path = output_root.join(CRYPTO_CAPTURE_STATE_FILE);
    let mut state = read_state_or_default(&state_path)?;
    validate_state_config(&state, &config)?;
    state.schema_version = CRYPTO_CAPTURE_HARNESS_SCHEMA_VERSION.to_string();
    state.domain = config.ingestor_config.domain.clone();
    state.interval_secs = config.interval_secs;
    let due_slot = request.now_ts / config.interval_secs;
    if state
        .captures
        .iter()
        .any(|record| record.due_slot == due_slot)
    {
        let report = persist_report(
            output_root,
            &state_path,
            CryptoCaptureDecisionKind::SkippedDuplicateInterval,
            due_slot,
            None,
            &state,
        )?;
        return Ok(CryptoCaptureHarnessRun {
            state_path,
            report_path: output_root.join(CRYPTO_CAPTURE_REPORT_FILE),
            state,
            report,
        });
    }

    let mut ingest_config = config.ingestor_config;
    ingest_config.captured_ts = request.now_ts;
    let run = runner.run_capture_cycle(
        store,
        register,
        request.vault_id,
        request.vault_salt,
        output_root,
        ingest_config,
    )?;
    let record = capture_record(&run, register, due_slot)?;
    state.captures.push(record.clone());
    write_state_readback(output_root, &state)?;
    let report = persist_report(
        output_root,
        &state_path,
        CryptoCaptureDecisionKind::Captured,
        due_slot,
        Some(record),
        &state,
    )?;
    Ok(CryptoCaptureHarnessRun {
        state_path,
        report_path: output_root.join(CRYPTO_CAPTURE_REPORT_FILE),
        state,
        report,
    })
}

pub fn join_crypto_capture_resolution<S>(
    store: &S,
    register: &mut PendingForecastRegister,
    output_root: &Path,
    resolution: &Resolution,
    voided: bool,
) -> Result<CryptoCaptureResolutionRun>
where
    S: PendingForecastLedgerStore,
{
    reject_forbidden_drive(output_root)?;
    let state_path = output_root.join(CRYPTO_CAPTURE_STATE_FILE);
    let mut state = read_state_or_default(&state_path)?;
    hydrate_register_from_state(&state, register);
    let join = join_resolution_to_pending_forecasts(store, register, resolution, voided)?;
    if !join.lookahead_blocked_forecast_ids.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_LOOKAHEAD,
            format!(
                "resolution {} at {} is not after all selected forecast timestamps: {:?}",
                resolution.condition_id,
                resolution.resolved_ts,
                join.lookahead_blocked_forecast_ids
            ),
        ));
    }
    if join.work_items.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_NO_MATURED_PAIR,
            format!(
                "resolution {} produced no matured pre-resolution crypto pairs",
                resolution.condition_id
            ),
        ));
    }
    update_state_entries(&mut state, register);
    let record = matured_record(&state, resolution, voided, &join)?;
    if !state
        .matured_resolutions
        .iter()
        .any(|existing| existing.resolution_id == record.resolution_id)
    {
        state.matured_resolutions.push(record.clone());
    }
    write_state_readback(output_root, &state)?;
    let corpus_path = write_corpus_readback(output_root, &state)?;
    Ok(CryptoCaptureResolutionRun {
        state_path,
        corpus_path,
        state,
        join,
        record,
    })
}

pub fn read_crypto_capture_state(path: &Path) -> Result<CryptoCaptureHarnessState> {
    read_json(path)
}

fn validate_config(config: &CryptoCaptureHarnessConfig) -> Result<()> {
    if config.interval_secs == 0 {
        return Err(PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_INVALID_CONFIG,
            "crypto capture interval_secs must be greater than zero",
        ));
    }
    Ok(())
}

fn validate_state_config(
    state: &CryptoCaptureHarnessState,
    config: &CryptoCaptureHarnessConfig,
) -> Result<()> {
    if state.interval_secs != 0 && state.interval_secs != config.interval_secs {
        return Err(PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_INVALID_CONFIG,
            format!(
                "existing harness interval {} does not match requested {}",
                state.interval_secs, config.interval_secs
            ),
        ));
    }
    if !state.captures.is_empty() && state.domain != config.ingestor_config.domain {
        return Err(PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_INVALID_CONFIG,
            format!(
                "existing harness domain {} does not match requested {}",
                state.domain, config.ingestor_config.domain
            ),
        ));
    }
    Ok(())
}

fn read_state_or_default(path: &Path) -> Result<CryptoCaptureHarnessState> {
    if path.exists() {
        read_json(path)
    } else {
        Ok(CryptoCaptureHarnessState::default())
    }
}

fn write_state_readback(dir: &Path, state: &CryptoCaptureHarnessState) -> Result<PathBuf> {
    let path = write_json(dir, CRYPTO_CAPTURE_STATE_FILE, state)?;
    let readback: CryptoCaptureHarnessState = read_json(&path)?;
    if readback != *state {
        return Err(PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_READBACK,
            format!(
                "capture state {} did not read back as written",
                path.display()
            ),
        ));
    }
    Ok(path)
}

fn persist_report(
    dir: &Path,
    state_path: &Path,
    decision: CryptoCaptureDecisionKind,
    due_slot: u64,
    captured_record: Option<CryptoCaptureRecord>,
    state: &CryptoCaptureHarnessState,
) -> Result<CryptoCaptureHarnessReport> {
    let report = CryptoCaptureHarnessReport {
        schema_version: CRYPTO_CAPTURE_HARNESS_SCHEMA_VERSION.to_string(),
        source_of_truth: "durable crypto capture state JSON plus AsterVault Base/Ledger rows"
            .to_string(),
        state_path: state_path.display().to_string(),
        decision,
        due_slot,
        captured_record,
        capture_count_after: state.captures.len(),
        matured_pair_count_after: state
            .matured_resolutions
            .iter()
            .map(|record| record.pairs.len())
            .sum(),
    };
    let path = write_json(dir, CRYPTO_CAPTURE_REPORT_FILE, &report)?;
    let readback: CryptoCaptureHarnessReport = read_json(&path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_READBACK,
            format!(
                "capture report {} did not read back as written",
                path.display()
            ),
        ));
    }
    Ok(report)
}

fn capture_record(
    run: &CryptoIngestionRun,
    register: &PendingForecastRegister,
    due_slot: u64,
) -> Result<CryptoCaptureRecord> {
    let snapshots = run
        .snapshots
        .iter()
        .map(|snapshot| {
            let entry = register
                .entries
                .iter()
                .find(|entry| entry.forecast_id == snapshot.pending.forecast_id)
                .cloned()
                .ok_or_else(|| {
                    PolyError::diagnostics(
                        ERR_CRYPTO_CAPTURE_PENDING_ENTRY,
                        format!("missing pending entry {}", snapshot.pending.forecast_id),
                    )
                })?;
            Ok(CryptoCapturedSnapshotRef {
                cx_id: snapshot.put.cx_id.clone(),
                token_id: snapshot.put.token_id.clone(),
                forecast_id: snapshot.pending.forecast_id.clone(),
                forecast_artifact_path: snapshot.pending.forecast_artifact_path.clone(),
                forecast_artifact_blake3: snapshot.pending.forecast_artifact_blake3.clone(),
                outcome_index: entry.outcome_index,
                forecast_ts: entry.forecast_ts,
                pending_entry: entry,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let bytes = serde_json::to_vec(run).map_err(|err| {
        PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_READBACK,
            format!("serialize capture run for hash: {err}"),
        )
    })?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    Ok(CryptoCaptureRecord {
        capture_id: format!("crypto-capture-{due_slot}-{}", &hash[..16]),
        due_slot,
        captured_ts: run.captured_ts,
        market_id: run.market_id.clone(),
        condition_id: run.condition_id.clone(),
        token_count: run.token_count,
        run_hash_blake3: hash,
        snapshots,
    })
}

fn hydrate_register_from_state(
    state: &CryptoCaptureHarnessState,
    register: &mut PendingForecastRegister,
) {
    for entry in state.captures.iter().flat_map(|capture| {
        capture
            .snapshots
            .iter()
            .map(|snapshot| snapshot.pending_entry.clone())
    }) {
        if !register
            .entries
            .iter()
            .any(|existing| existing.forecast_id == entry.forecast_id)
        {
            register.entries.push(entry);
        }
    }
}

fn update_state_entries(state: &mut CryptoCaptureHarnessState, register: &PendingForecastRegister) {
    for snapshot in state
        .captures
        .iter_mut()
        .flat_map(|capture| capture.snapshots.iter_mut())
    {
        if let Some(entry) = register
            .entries
            .iter()
            .find(|entry| entry.forecast_id == snapshot.forecast_id)
        {
            snapshot.pending_entry = entry.clone();
        }
    }
}

fn matured_record(
    state: &CryptoCaptureHarnessState,
    resolution: &Resolution,
    voided: bool,
    join: &ResolutionJoinResult,
) -> Result<CryptoMaturedResolutionRecord> {
    let pairs = join
        .work_items
        .iter()
        .map(|item| pair_for_item(state, resolution, join, item))
        .collect::<Result<Vec<_>>>()?;
    Ok(CryptoMaturedResolutionRecord {
        resolution_id: join.resolution_id.clone(),
        condition_id: resolution.condition_id.clone(),
        resolved_ts: resolution.resolved_ts,
        voided,
        idempotent_replay: join.idempotent_replay,
        work_item_count: join.work_items.len(),
        join_ledger_seq: join.ledger_seq,
        pairs,
    })
}

fn pair_for_item(
    state: &CryptoCaptureHarnessState,
    resolution: &Resolution,
    join: &ResolutionJoinResult,
    item: &PendingForecastWorkItem,
) -> Result<CryptoPreResolutionPair> {
    let snapshot = state
        .captures
        .iter()
        .flat_map(|capture| capture.snapshots.iter())
        .find(|snapshot| snapshot.forecast_id == item.forecast_id)
        .ok_or_else(|| {
            PolyError::diagnostics(
                ERR_CRYPTO_CAPTURE_PENDING_ENTRY,
                format!("missing captured snapshot for {}", item.forecast_id),
            )
        })?;
    let actual_win = item.actual_win.ok_or_else(|| {
        PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_NO_MATURED_PAIR,
            format!(
                "voided forecast {} cannot become a scored pair",
                item.forecast_id
            ),
        )
    })?;
    Ok(CryptoPreResolutionPair {
        condition_id: item.condition_id.clone(),
        token_id: item.token_id.clone(),
        outcome_index: item.outcome_index,
        snapshot_cx_id: snapshot.cx_id.clone(),
        forecast_id: item.forecast_id.clone(),
        forecast_ts: snapshot.forecast_ts,
        p_model: item.p_model,
        confidence: item.confidence,
        resolution_id: join.resolution_id.clone(),
        resolved_ts: resolution.resolved_ts,
        actual_win,
    })
}

fn write_corpus_readback(dir: &Path, state: &CryptoCaptureHarnessState) -> Result<PathBuf> {
    let pairs = state
        .matured_resolutions
        .iter()
        .flat_map(|record| record.pairs.clone())
        .collect::<Vec<_>>();
    let path = write_json(dir, CRYPTO_PRE_RESOLUTION_CORPUS_FILE, &pairs)?;
    let readback: Vec<CryptoPreResolutionPair> = read_json(&path)?;
    if readback != pairs {
        return Err(PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_READBACK,
            format!(
                "pre-resolution corpus {} did not read back as written",
                path.display()
            ),
        ));
    }
    Ok(path)
}
