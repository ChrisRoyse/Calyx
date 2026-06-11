use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use calyx_anneal::{
    CandidateLens, DifferentiationGate, LensProfiler, PairNMI, describe_gate_outcome,
};
use calyx_core::{CalyxError, Clock, Constellation, LensId, Result as CalyxResult};
use calyx_registry::{
    CapabilityCard, CostMetrics, CoverageMetrics, LensHealth, MetricSource, SeparationMetrics,
    SpreadMetrics,
};
use serde::Deserialize;
use serde_json::json;

const CALYX_ASSAY_INVALID_METRIC: &str = "CALYX_ASSAY_INVALID_METRIC";

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let request = LensProposalLogRequest::parse(args)?;
    let fixture_bytes = fs::read(&request.fixture).map_err(|error| {
        format!(
            "{CALYX_ASSAY_INVALID_METRIC}: read fixture {}: {error}",
            request.fixture.display()
        )
    })?;
    let fixture = serde_json::from_slice::<Fixture>(&fixture_bytes).map_err(|error| {
        format!(
            "{CALYX_ASSAY_INVALID_METRIC}: parse fixture {}: {error}",
            request.fixture.display()
        )
    })?;
    let mut entries = Vec::new();
    for event in fixture.events {
        let entry = run_event(fixture.clock_ts, event)?;
        entries.push(entry);
    }
    if request.last < entries.len() {
        entries.drain(0..entries.len() - request.last);
    }
    let readback = json!({
        "source_of_truth": "fixture JSON bytes read from fixture_path; GateOutcome recomputed by calyx anneal lens-proposal-log",
        "fixture_path": request.fixture.display().to_string(),
        "fixture_len": fixture_bytes.len(),
        "fixture_blake3": blake3::hash(&fixture_bytes).to_hex().to_string(),
        "last": request.last,
        "entries": entries,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}

struct LensProposalLogRequest {
    fixture: PathBuf,
    last: usize,
}

impl LensProposalLogRequest {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut fixture = None;
        let mut last = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--fixture" => {
                    fixture = args.get(idx + 1).map(PathBuf::from);
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
        Ok(Self {
            fixture: fixture.ok_or_else(|| "lens-proposal-log requires --fixture".to_string())?,
            last,
        })
    }
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
        Ok(card(self.lens_id, self.bits as f32, corpus_sample.len()))
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

fn card(lens_id: LensId, bits: f32, probe_count: usize) -> CapabilityCard {
    CapabilityCard {
        lens_id,
        probe_count,
        signal: Some(bits),
        signal_source: MetricSource::AssayStore,
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
