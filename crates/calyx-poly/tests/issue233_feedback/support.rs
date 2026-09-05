use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Anchor, AnchorKind, AnchorValue, FixedClock, VaultStore};
use calyx_ledger::{DirectoryLedgerStore, EntryKind, LedgerAppender, LedgerCfStore};
use calyx_poly::{
    BlendRelearningRequest, BlendWeightObservation, CalibrationRefitObservation,
    CalibrationRefitRequest, ComponentKind, FeedbackBackfillInput, FeedbackControllerCycleRequest,
    FeedbackLearningRequest, FeedbackMetaLearningRequest, FeedbackResolutionInput,
    ForecastScoreRequest, ForecastSource, MetaLearningEffect, PendingForecastEntry,
    PendingForecastRegister, PendingForecastStatus, ProxyKind, Resolution, ResolvedOutcome,
    SelfEvolutionGuardrailRequest, SelfEvolutionMetrics, SelfEvolutionTripwires, proxy_anchor,
    record_pending_forecast,
};
use serde_json::{Value, json};

use super::support::{reset_dir, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue233-feedback-controller";
const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const NOW: u64 = 1_785_700_000;

pub fn cycle_request<'a>(
    report_dir: &'a Path,
    score_root: &'a Path,
    cycle_id: &'a str,
    resolutions: Vec<FeedbackResolutionInput>,
    score_requests: Vec<ForecastScoreRequest>,
    backfills: Vec<FeedbackBackfillInput>,
    learning: Option<FeedbackLearningRequest<'a>>,
) -> FeedbackControllerCycleRequest<'a> {
    FeedbackControllerCycleRequest {
        cycle_id,
        report_dir,
        score_root,
        resolutions,
        score_requests,
        backfills,
        learning,
    }
}

pub fn learning<'a>(paths: &'a LearningPaths, rejected: bool) -> FeedbackLearningRequest<'a> {
    FeedbackLearningRequest {
        blend: BlendRelearningRequest {
            out_dir: &paths.blend_dir,
            domain: "crypto",
            horizon_bucket: "1h",
            as_of_millis: NOW * 1000,
            min_samples_per_component: 2,
            observations: blend_observations(),
        },
        calibration: CalibrationRefitRequest {
            out_dir: &paths.calibration_dir,
            domain: "crypto",
            horizon_bucket: "1h",
            previous_version: None,
            as_of_millis: NOW * 1000,
            observations: calibration_observations(),
        },
        guardrail: guardrail_request(paths, rejected),
        meta: FeedbackMetaLearningRequest {
            ledger_dir: &paths.meta_dir,
            change_id: if rejected {
                "change233rejected"
            } else {
                "change233approved"
            },
            changed_surface: "forecast_pipeline",
            rationale: if rejected {
                "shadow replay rejected candidate"
            } else {
                "shadow replay improved held-out score"
            },
            responsible_actor: "calyx-poly-feedback-controller",
            effect: if rejected {
                MetaLearningEffect {
                    objective_score_delta: -0.01,
                    kernel_recall_delta: -0.02,
                    guard_far_delta: 0.0,
                    p95_latency_delta_ms: 5.0,
                }
            } else {
                MetaLearningEffect {
                    objective_score_delta: 0.03,
                    kernel_recall_delta: 0.01,
                    guard_far_delta: -0.005,
                    p95_latency_delta_ms: -4.0,
                }
            },
            rollback_artifact_path: &paths.rollback_path,
            fsv_artifact_path: &paths.fsv_path,
        },
    }
}

pub fn fixture(root: &Path, name: &str) -> Fixture {
    let case_root = root.join(name);
    reset_dir(&case_root);
    let vault = AsterVault::new_durable(
        case_root.join("vault"),
        VAULT_ID.parse().unwrap(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open vault");
    let score_ledger_dir = case_root.join("score-ledger");
    let score_ledger = LedgerAppender::open(
        DirectoryLedgerStore::open(&score_ledger_dir).expect("open score ledger"),
        FixedClock::new(NOW),
    )
    .expect("open appender");
    Fixture {
        score_root: case_root.join("scores"),
        report_dir: case_root.join("controller-report"),
        root: case_root,
        vault,
        register: PendingForecastRegister::default(),
        score_ledger_dir,
        score_ledger,
    }
}

pub fn learning_paths(root: &Path, name: &str) -> LearningPaths {
    let dir = root.join("learning").join(name);
    fs::create_dir_all(&dir).expect("create learning dir");
    let rollback_path = dir.join("rollback.json");
    let repro_path = dir.join("reproduction.json");
    let fsv_path = dir.join("fsv-source.json");
    fs::write(&rollback_path, br#"{"restore":"prior-state"}"#).expect("rollback");
    fs::write(&repro_path, br#"{"replay":"shadow"}"#).expect("repro");
    fs::write(&fsv_path, br#"{"source":"issue233"}"#).expect("fsv");
    LearningPaths {
        blend_dir: dir.join("blend"),
        calibration_dir: dir.join("calibration"),
        guardrail_dir: dir.join("guardrail"),
        meta_dir: dir.join("meta"),
        rollback_path,
        repro_path,
        fsv_path,
    }
}

pub fn record(
    vault: &AsterVault,
    register: &mut PendingForecastRegister,
    entry: PendingForecastEntry,
) -> u64 {
    let seq = record_pending_forecast(vault, register, entry)
        .expect("record pending")
        .seq;
    vault.flush().expect("flush pending");
    seq
}

pub fn forecast(
    id: &str,
    condition: &str,
    outcome_index: u32,
    forecast_ts: u64,
) -> PendingForecastEntry {
    PendingForecastEntry {
        forecast_id: id.to_string(),
        source: ForecastSource::CalyxNative,
        condition_id: condition.to_string(),
        token_id: format!("{condition}-token-{outcome_index}"),
        outcome_index,
        domain: "crypto".to_string(),
        horizon_bucket: "1h".to_string(),
        forecast_version: 1,
        p_model: 0.8,
        confidence: 0.6,
        forecast_ts,
        provenance_hash: HASH_A.to_string(),
        status: PendingForecastStatus::Pending,
        registered_ledger_seq: None,
        terminal_ledger_seq: None,
        terminal_resolution_id: None,
        terminal_actual_win: None,
    }
}

pub fn input(
    condition: &str,
    winning: u32,
    resolved_ts: u64,
    voided: bool,
    disputed: bool,
) -> FeedbackResolutionInput {
    FeedbackResolutionInput {
        resolution: Resolution {
            condition_id: condition.to_string(),
            winning_outcome_index: winning,
            winning_label: if winning == 0 { "YES" } else { "NO" }.to_string(),
            resolved_ts,
            source: "uma-onchain".to_string(),
            disputed,
        },
        voided,
    }
}

pub fn score(
    score_id: &str,
    forecast_id: &str,
    condition: &str,
    actual_win: bool,
) -> ForecastScoreRequest {
    ForecastScoreRequest {
        score_id: score_id.to_string(),
        forecast_id: forecast_id.to_string(),
        forecast_version: 1,
        current_forecast_version: 1,
        market_id: condition.to_string(),
        outcome_id: format!("{condition}-token-0"),
        source: ForecastSource::CalyxNative,
        provider: None,
        probability: 0.8,
        confidence: 0.6,
        forecast_ts: 100,
        scored_ts: 220,
        horizon_secs: 100,
        sufficiency_state: "sufficient".to_string(),
        previous_probability: Some(0.7),
        forecast_artifact_hash: HASH_B.to_string(),
        outcome: ResolvedOutcome {
            outcome_id: format!("{condition}-token-0"),
            resolved: true,
            actual_win,
            resolved_ts: 200,
            source: "uma-onchain".to_string(),
            version: 1,
        },
        calibration_bin_count: 10,
    }
}

pub fn backfill(name: &str, proxy_value: bool, resolved_value: bool) -> FeedbackBackfillInput {
    FeedbackBackfillInput {
        name: name.to_string(),
        proxy: proxy_anchor(ProxyKind::Up1h, proxy_value, 0.6, 100).expect("proxy anchor"),
        resolved: Anchor {
            kind: AnchorKind::TestPass,
            value: AnchorValue::Bool(resolved_value),
            source: "uma:uma-onchain:YES".to_string(),
            observed_at: 200_000,
            confidence: 1.0,
        },
    }
}

pub fn vault_payload(vault: &AsterVault, seq: u64) -> Value {
    let row = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(seq))
        .expect("read ledger")
        .expect("ledger present");
    let ledger = calyx_ledger::decode(&row).expect("decode ledger");
    serde_json::from_slice(&ledger.payload).expect("decode payload")
}

pub fn score_payloads(dir: &Path) -> Vec<Value> {
    let store = DirectoryLedgerStore::open(dir).expect("open score ledger readback");
    store
        .scan()
        .expect("scan score ledger")
        .into_iter()
        .map(|row| {
            let entry = calyx_ledger::decode(&row.bytes).expect("decode score ledger");
            assert_eq!(entry.kind, EntryKind::Score);
            serde_json::from_slice(&entry.payload).expect("payload")
        })
        .collect()
}

pub fn persist_case(root: &Path, value: Value) -> Value {
    let path = root.join("readback.json");
    write_json(&path, &value);
    let expected = serde_json::to_vec_pretty(&value).expect("serialize case");
    let actual = fs::read(&path).expect("read case");
    assert_eq!(actual, expected);
    json!({
        "path": path.display().to_string(),
        "readback_equal": true,
        "value_blake3": blake3::hash(&actual).to_hex().to_string()
    })
}

fn guardrail_request<'a>(
    paths: &'a LearningPaths,
    rejected: bool,
) -> SelfEvolutionGuardrailRequest<'a> {
    SelfEvolutionGuardrailRequest {
        out_dir: &paths.guardrail_dir,
        change_id: if rejected {
            "change233rejected"
        } else {
            "change233approved"
        },
        rationale: "feedback-controller shadow replay",
        baseline: SelfEvolutionMetrics {
            kernel_recall_ratio: 0.96,
            guard_far_ratio: 0.02,
            p95_latency_ms: 100.0,
        },
        candidate: if rejected {
            SelfEvolutionMetrics {
                kernel_recall_ratio: 0.93,
                guard_far_ratio: 0.02,
                p95_latency_ms: 105.0,
            }
        } else {
            SelfEvolutionMetrics {
                kernel_recall_ratio: 0.97,
                guard_far_ratio: 0.015,
                p95_latency_ms: 96.0,
            }
        },
        tripwires: SelfEvolutionTripwires {
            min_kernel_recall_ratio: 0.95,
            max_recall_regression: 0.01,
            max_guard_far_ratio: 0.05,
            max_guard_far_increase: 0.01,
            max_p95_latency_ms: 150.0,
            max_latency_increase_ratio: 1.10,
        },
        rollback_artifact_path: &paths.rollback_path,
        reproduction_plan_path: &paths.repro_path,
    }
}

fn blend_observations() -> Vec<BlendWeightObservation> {
    vec![
        blend_obs(ComponentKind::KnnBaseRate, 0.8, true),
        blend_obs(ComponentKind::KnnBaseRate, 0.2, false),
        blend_obs(ComponentKind::BitsVote, 0.75, true),
        blend_obs(ComponentKind::BitsVote, 0.25, false),
    ]
}

fn calibration_observations() -> Vec<CalibrationRefitObservation> {
    let mut rows = Vec::new();
    for i in 0..15 {
        rows.push(CalibrationRefitObservation {
            p_raw: 0.6,
            outcome_yes: i % 5 != 0,
            resolved_at_millis: (NOW - 10) * 1000,
        });
        rows.push(CalibrationRefitObservation {
            p_raw: 0.4,
            outcome_yes: i % 5 == 0,
            resolved_at_millis: (NOW - 10) * 1000,
        });
    }
    rows
}

fn blend_obs(component: ComponentKind, p_yes: f64, outcome_yes: bool) -> BlendWeightObservation {
    BlendWeightObservation {
        component,
        p_yes,
        outcome_yes,
        observed_at_millis: (NOW - 1) * 1000,
    }
}

pub struct Fixture {
    pub root: PathBuf,
    pub score_root: PathBuf,
    pub report_dir: PathBuf,
    pub vault: AsterVault,
    pub register: PendingForecastRegister,
    pub score_ledger_dir: PathBuf,
    pub score_ledger: LedgerAppender<DirectoryLedgerStore, FixedClock>,
}

pub struct LearningPaths {
    pub blend_dir: PathBuf,
    pub calibration_dir: PathBuf,
    pub guardrail_dir: PathBuf,
    pub meta_dir: PathBuf,
    pub rollback_path: PathBuf,
    pub repro_path: PathBuf,
    pub fsv_path: PathBuf,
}
