use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use calyx_anneal::{
    AnnealLedgerAction, CandidateLens, DifferentiationGate, LensProfiler, PairNMI,
    decode_anneal_ledger_payload, describe_gate_outcome, record_from_entry,
};
use calyx_core::{CalyxError, Clock, Constellation, LedgerRef, LensId, Result as CalyxResult};
use calyx_ledger::{EntryKind, LedgerCfStore, decode};
use calyx_registry::{
    CapabilityCard, CapabilitySignalKind, CostMetrics, CoverageMetrics, LensHealth, MetricSource,
    SeparationMetrics, SpreadMetrics,
};
use serde::Deserialize;
use serde_json::json;

use crate::cf_read::hex_bytes as hex;
use crate::ledger_store::AsterLedgerCfStore;

const CALYX_ASSAY_INVALID_METRIC: &str = "CALYX_ASSAY_INVALID_METRIC";

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = LensProposalLogRequest::parse(args)?;
    let readback = match request.source {
        LensProposalLogSource::Fixture(fixture) => read_fixture_entries(fixture, request.last)?,
        LensProposalLogSource::Vault(vault) => read_vault_entries(vault, request.last)?,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}

struct LensProposalLogRequest {
    source: LensProposalLogSource,
    last: usize,
}

enum LensProposalLogSource {
    Fixture(PathBuf),
    Vault(PathBuf),
}

impl LensProposalLogRequest {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut fixture = None;
        let mut vault = None;
        let mut last = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--fixture" => {
                    fixture = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                "--vault" => {
                    vault = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                "--last" => {
                    last = Some(
                        args.get(idx + 1)
                            .ok_or_else(|| "--last requires a value".to_string())?
                            .parse::<usize>()
                            .map_err(|error| format!("invalid --last: {error}"))?,
                    );
                    idx += 2;
                }
                other => return Err(format!("unknown lens-proposal-log arg: {other}")),
            }
        }
        let last = last.unwrap_or(5);
        if last == 0 {
            return Err("--last must be positive".to_string());
        }
        let source = match (fixture, vault) {
            (Some(_), Some(_)) => {
                return Err(
                    "lens-proposal-log accepts either --fixture or --vault, not both".into(),
                );
            }
            (Some(path), None) => LensProposalLogSource::Fixture(path),
            (None, Some(path)) => LensProposalLogSource::Vault(path),
            (None, None) => {
                return Err("lens-proposal-log requires --fixture <json> or --vault <dir>".into());
            }
        };
        Ok(Self { source, last })
    }
}

fn read_fixture_entries(fixture_path: PathBuf, last: usize) -> Result<serde_json::Value, String> {
    let fixture_bytes = fs::read(&fixture_path).map_err(|error| {
        format!(
            "{CALYX_ASSAY_INVALID_METRIC}: read fixture {}: {error}",
            fixture_path.display()
        )
    })?;
    let fixture = serde_json::from_slice::<Fixture>(&fixture_bytes).map_err(|error| {
        format!(
            "{CALYX_ASSAY_INVALID_METRIC}: parse fixture {}: {error}",
            fixture_path.display()
        )
    })?;
    let mut entries = Vec::new();
    for event in fixture.events {
        entries.push(run_event(fixture.clock_ts, event)?);
    }
    if last < entries.len() {
        entries.drain(0..entries.len() - last);
    }
    Ok(json!({
        "source_of_truth": "fixture JSON bytes read from fixture_path; GateOutcome recomputed by calyx anneal lens-proposal-log",
        "fixture_path": fixture_path.display().to_string(),
        "fixture_len": fixture_bytes.len(),
        "fixture_blake3": blake3::hash(&fixture_bytes).to_hex().to_string(),
        "last": last,
        "entries": entries,
    }))
}

fn read_vault_entries(vault: PathBuf, last: usize) -> Result<serde_json::Value, String> {
    let store = AsterLedgerCfStore::open(&vault).map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    for row in store.scan().map_err(|error| error.to_string())? {
        let entry = decode(&row.bytes).map_err(|error| error.to_string())?;
        if entry.seq != row.seq {
            return Err(format!(
                "ledger row seq {} decodes to entry seq {}",
                row.seq, entry.seq
            ));
        }
        if entry.kind != EntryKind::Anneal {
            continue;
        }
        let anneal =
            decode_anneal_ledger_payload(&entry.payload).map_err(|error| error.to_string())?;
        if !matches!(
            anneal.action,
            AnnealLedgerAction::LensAdmitted | AnnealLedgerAction::LensRejected
        ) {
            continue;
        }
        let ledger_ref = LedgerRef {
            seq: entry.seq,
            hash: entry.entry_hash,
        };
        let readback = record_from_entry(ledger_ref, anneal)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "proposal ledger action did not produce history entry".to_string())?;
        entries.push(json!({
            "seq": row.seq,
            "entry_hash": hex(&entry.entry_hash),
            "payload_hex": hex(&entry.payload),
            "record": readback.record,
        }));
    }
    if last < entries.len() {
        entries.drain(0..entries.len() - last);
    }
    Ok(json!({
        "source_of_truth": "Aster ledger CF rows plus WAL replay under <vault>/cf/ledger and <vault>/wal",
        "vault": vault.display().to_string(),
        "last": last,
        "entries": entries,
    }))
}

#[derive(Deserialize)]
struct Fixture {
    #[serde(default)]
    clock_ts: u64,
    events: Vec<FixtureEvent>,
}

#[derive(Deserialize)]
struct FixtureEvent {
    seq: u64,
    candidate: CandidateLens,
    candidate_lens_id: LensId,
    profile_bits: MetricFixture,
    #[serde(default = "default_profile_signal_kind")]
    profile_signal_kind: CapabilitySignalKind,
    #[serde(default)]
    profile_elapsed_ms: u64,
    #[serde(default)]
    panel: Vec<LensId>,
    #[serde(default)]
    correlations: Vec<FixtureCorrelation>,
}

#[derive(Deserialize)]
struct FixtureCorrelation {
    lens_id: LensId,
    corr: MetricFixture,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum MetricFixture {
    Number(f64),
    String(String),
}

impl MetricFixture {
    fn value(&self) -> Result<f64, String> {
        match self {
            Self::Number(value) => Ok(*value),
            Self::String(value) if value.eq_ignore_ascii_case("nan") => Ok(f64::NAN),
            Self::String(value) if value.eq_ignore_ascii_case("inf") => Ok(f64::INFINITY),
            Self::String(value) if value.eq_ignore_ascii_case("-inf") => Ok(f64::NEG_INFINITY),
            Self::String(value) => value
                .parse::<f64>()
                .map_err(|error| format!("{CALYX_ASSAY_INVALID_METRIC}: parse metric: {error}")),
        }
    }
}

fn run_event(clock_ts: u64, event: FixtureEvent) -> Result<serde_json::Value, String> {
    let clock = SharedClock::new(clock_ts);
    let profiler = FixtureProfiler {
        lens_id: event.candidate_lens_id,
        bits: event.profile_bits.value()?,
        signal_kind: event.profile_signal_kind,
        elapsed_ms: event.profile_elapsed_ms,
        clock: clock.inner(),
    };
    let nmi = FixtureNmi::from_rows(event.correlations)?;
    let gate = DifferentiationGate::new(&clock);
    let outcome = gate
        .gate(&event.candidate, &event.panel, &profiler, &nmi, &[])
        .map_err(format_calyx_error)?;
    Ok(json!({
        "seq": event.seq,
        "candidate_lens_id": event.candidate_lens_id,
        "panel": event.panel,
        "outcome_description": describe_gate_outcome(&outcome),
        "outcome": outcome,
    }))
}

#[derive(Clone)]
struct SharedClock {
    now: Arc<AtomicU64>,
}

impl SharedClock {
    fn new(ts: u64) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(ts)),
        }
    }

    fn inner(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.now)
    }
}

impl Clock for SharedClock {
    fn now(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}

struct FixtureProfiler {
    lens_id: LensId,
    bits: f64,
    signal_kind: CapabilitySignalKind,
    elapsed_ms: u64,
    clock: Arc<AtomicU64>,
}

impl LensProfiler for FixtureProfiler {
    fn profile(
        &self,
        _candidate: &CandidateLens,
        corpus_sample: &[Constellation],
    ) -> CalyxResult<CapabilityCard> {
        self.clock.fetch_add(self.elapsed_ms, Ordering::SeqCst);
        Ok(card(
            self.lens_id,
            self.bits as f32,
            self.signal_kind,
            corpus_sample.len(),
        ))
    }
}

struct FixtureNmi {
    correlations: BTreeMap<LensId, f64>,
}

impl FixtureNmi {
    fn from_rows(rows: Vec<FixtureCorrelation>) -> Result<Self, String> {
        let mut correlations = BTreeMap::new();
        for row in rows {
            correlations.insert(row.lens_id, row.corr.value()?);
        }
        Ok(Self { correlations })
    }
}

impl PairNMI for FixtureNmi {
    fn lens_embeddings(
        &self,
        lens: &LensId,
        _corpus_sample: &[Constellation],
    ) -> CalyxResult<Vec<Vec<f32>>> {
        let corr = *self.correlations.get(lens).ok_or_else(|| CalyxError {
            code: CALYX_ASSAY_INVALID_METRIC,
            message: format!("missing fixture correlation for panel lens {lens}"),
            remediation: "repair lens proposal log fixture",
        })?;
        Ok(vec![vec![corr as f32]])
    }

    fn nmi(&self, _lens_a: &LensId, lens_b_embeddings: &[Vec<f32>]) -> CalyxResult<f64> {
        lens_b_embeddings
            .first()
            .and_then(|row| row.first())
            .copied()
            .map(f64::from)
            .ok_or_else(|| CalyxError {
                code: CALYX_ASSAY_INVALID_METRIC,
                message: "empty fixture NMI embeddings".to_string(),
                remediation: "repair lens proposal log fixture",
            })
    }
}

fn card(
    lens_id: LensId,
    bits: f32,
    signal_kind: CapabilitySignalKind,
    probe_count: usize,
) -> CapabilityCard {
    CapabilityCard {
        lens_id,
        probe_count,
        signal: Some(bits),
        signal_source: MetricSource::AssayStore,
        signal_kind,
        signal_reliability: None,
        proxy_signal: bits,
        differentiation: None,
        differentiation_source: MetricSource::AssayPending,
        proxy_differentiation: 0.0,
        spread: SpreadMetrics {
            participation_ratio: 1.0,
            normalized_participation_ratio: 1.0,
            stable_rank: 1.0,
            total_variance: 1.0,
            mean_pairwise_distance: 1.0,
        },
        separation: SeparationMetrics {
            score: bits,
            silhouette: bits,
            mean_pairwise_distance: 1.0,
            labeled_groups: 2,
            used_labels: true,
        },
        cost: CostMetrics {
            total_ms: 1.0,
            ms_per_input: 1.0,
            vram_bytes: 0,
            ram_bytes: 0,
            batch_ceiling: 1_000,
        },
        coverage: CoverageMetrics {
            requested: probe_count,
            measured: probe_count,
            failed: 0,
            rate: 1.0,
        },
        health: LensHealth::Loaded,
        low_spread: false,
    }
}

fn format_calyx_error(error: CalyxError) -> String {
    format!("{}: {} ({})", error.code, error.message, error.remediation)
}

fn default_profile_signal_kind() -> CapabilitySignalKind {
    CapabilitySignalKind::LearnedEncoder
}
