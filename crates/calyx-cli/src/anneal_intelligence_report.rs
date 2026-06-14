use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use calyx_anneal::{
    CALYX_ANNEAL_J_SYNTHETIC_RECURSION, DEFAULT_J_DOMAIN, GradientCandidate, IntelligenceGradient,
    JGeneratedPositiveCredit, JMetricSources, JObjectiveContext, JTerms, JValue, JWeights,
    compute_j, format_report, intelligence_report, latest_intelligence_report_snapshot,
    read_goodhart_state_from_vault, read_intelligence_report_snapshot,
    read_objective_weights_from_vault, report_diff, to_json, write_gradient_snapshot,
    write_intelligence_report_snapshot,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Clock, FixedClock, SystemClock, VaultId};
use serde::Deserialize;
use serde_json::json;

use crate::cf_read::hex_bytes;

const REPORT_VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const REPORT_VAULT_SALT: &[u8] = b"calyx-anneal-intelligence-report";

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = IntelligenceReportRequest::parse(args)?;
    let fixture_bytes = fs::read(&request.fixture).map_err(|error| {
        format!(
            "CALYX_ANNEAL_J_INVALID_METRIC: read fixture {}: {error}",
            request.fixture.display()
        )
    })?;
    let fixture = serde_json::from_slice::<Fixture>(&fixture_bytes).map_err(|error| {
        format!(
            "CALYX_ANNEAL_J_INVALID_METRIC: parse fixture {}: {error}",
            request.fixture.display()
        )
    })?;
    let (weights, weights_source) = request.resolve_weights(&fixture)?;
    let (goodhart_penalty, goodhart_state_source) = request.resolve_goodhart_penalty()?;
    let goodhart_state = calyx_anneal::GoodhartState {
        p_goodhart: goodhart_penalty,
    };
    let context = JObjectiveContext {
        domain: fixture
            .domain
            .clone()
            .unwrap_or_else(|| DEFAULT_J_DOMAIN.to_string()),
        panel_len: fixture.panel_len,
        weights,
        goodhart_penalty,
    };
    let j_value = match compute_j(&context, &fixture.metrics) {
        Ok(value) => value,
        Err(error) if error.code == CALYX_ANNEAL_J_SYNTHETIC_RECURSION => {
            return Err(error.into());
        }
        Err(_) => unavailable_j_value(weights, goodhart_penalty),
    };
    let gradient = request.resolve_gradient(&fixture, &j_value)?;
    let report = intelligence_report(
        &context,
        &fixture.metrics,
        &gradient.gradient,
        &goodhart_state,
        fixture.goodhart_last.clone(),
        gradient.clock.as_ref(),
    );
    let persisted = request.persist_report(&report)?;
    let readback = json!({
        "source_of_truth": "fixture JSON bytes read by calyx anneal intelligence-report",
        "fixture_path": request.fixture.display().to_string(),
        "fixture_len": fixture_bytes.len(),
        "fixture_blake3": blake3::hash(&fixture_bytes).to_hex().to_string(),
        "weights_source": weights_source,
        "goodhart_state_source": goodhart_state_source,
        "context": context,
        "raw_metrics": fixture.metrics,
        "j_value": to_json(&report),
        "human_report": format_report(&report),
        "anneal_report_state_source": persisted.state_source,
        "anneal_report_key_hex": persisted.key_hex,
        "anneal_report_persisted_readback": persisted.readback,
        "report_diff_from_previous": persisted.diff,
        "gradient_state_source": gradient.state_source,
        "gradient_refresh": gradient.refresh,
        "gradient": gradient.snapshot.gradient,
        "next_best_action": gradient.snapshot.next_best_action,
        "gradient_warnings": gradient.snapshot.warnings,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}

struct IntelligenceReportRequest {
    fixture: PathBuf,
    vault: Option<PathBuf>,
}

impl IntelligenceReportRequest {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut fixture = None;
        let mut vault = None;
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
                other => return Err(format!("unknown intelligence-report arg: {other}")),
            }
        }
        Ok(Self {
            fixture: fixture
                .ok_or_else(|| "intelligence-report requires --fixture <json>".to_string())?,
            vault,
        })
    }

    fn resolve_weights(&self, fixture: &Fixture) -> Result<(JWeights, String), String> {
        if let Some(weights) = fixture.weights {
            return Ok((weights, "fixture.weights".to_string()));
        }
        if let Some(vault) = &self.vault {
            let weights =
                read_objective_weights_from_vault(vault).map_err(|error| error.to_string())?;
            return Ok((
                weights,
                format!("{}/.anneal/j_weights.toml", vault.display()),
            ));
        }
        Ok((
            JWeights::default(),
            "default PRD27 unit weights".to_string(),
        ))
    }

    fn resolve_goodhart_penalty(&self) -> Result<(f64, String), String> {
        if let Some(vault) = &self.vault {
            let state = read_goodhart_state_from_vault(vault).map_err(|error| error.to_string())?;
            return Ok((
                state.p_goodhart,
                format!("{}/.anneal/goodhart_state.toml", vault.display()),
            ));
        }
        Ok((0.0, "default no vault Goodhart state".to_string()))
    }

    fn resolve_gradient(
        &self,
        fixture: &Fixture,
        j_value: &JValue,
    ) -> Result<GradientReportState, String> {
        let clock: Arc<dyn Clock> = fixture
            .gradient_ts
            .map(|ts| Arc::new(FixedClock::new(ts)) as Arc<dyn Clock>)
            .unwrap_or_else(|| Arc::new(SystemClock));
        let mut gradient = IntelligenceGradient::new(j_value.clone(), clock.clone())
            .with_budget_units(fixture.gradient_budget_units.unwrap_or(u64::MAX));
        let refresh = gradient.refresh(fixture.gradient_candidates.clone());
        let snapshot = gradient.snapshot(5);
        let state_source = if let Some(vault) = &self.vault {
            let path =
                write_gradient_snapshot(vault, &snapshot).map_err(|error| error.to_string())?;
            path.display().to_string()
        } else {
            "not persisted without --vault".to_string()
        };
        Ok(GradientReportState {
            gradient,
            clock,
            refresh,
            snapshot,
            state_source,
        })
    }

    fn persist_report(
        &self,
        report: &calyx_anneal::IntelligenceReport,
    ) -> Result<PersistedReportState, String> {
        let Some(vault_path) = &self.vault else {
            return Ok(PersistedReportState {
                state_source: "not persisted without --vault".to_string(),
                key_hex: None,
                readback: None,
                diff: None,
            });
        };
        let vault_id = REPORT_VAULT_ID.parse::<VaultId>().map_err(|error| {
            format!("CALYX_ANNEAL_REPORT_INVALID_ROW: parse report vault id: {error}")
        })?;
        let vault = AsterVault::new_durable(
            vault_path,
            vault_id,
            REPORT_VAULT_SALT.to_vec(),
            VaultOptions::default(),
        )
        .map_err(|error| error.to_string())?;
        let previous =
            latest_intelligence_report_snapshot(&vault).map_err(|error| error.to_string())?;
        let key = write_intelligence_report_snapshot(&vault, report)
            .map_err(|error| error.to_string())?;
        let readback = read_intelligence_report_snapshot(&vault, report.ts)
            .map_err(|error| error.to_string())?
            .map(|stored| to_json(&stored));
        let diff = previous
            .as_ref()
            .map(|before| json!(report_diff(before, report)));
        Ok(PersistedReportState {
            state_source: format!("{}/cf/anneal_report", vault_path.display()),
            key_hex: Some(hex_bytes(&key)),
            readback,
            diff,
        })
    }
}

struct GradientReportState {
    gradient: IntelligenceGradient,
    clock: Arc<dyn Clock>,
    refresh: calyx_anneal::GradientRefreshReport,
    snapshot: calyx_anneal::GradientSnapshot,
    state_source: String,
}

struct PersistedReportState {
    state_source: String,
    key_hex: Option<String>,
    readback: Option<serde_json::Value>,
    diff: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    #[serde(default)]
    domain: Option<String>,
    panel_len: usize,
    #[serde(default)]
    weights: Option<JWeights>,
    #[serde(default)]
    gradient_candidates: Vec<GradientCandidate>,
    #[serde(default)]
    gradient_budget_units: Option<u64>,
    #[serde(default)]
    gradient_ts: Option<u64>,
    #[serde(default)]
    goodhart_last: Option<calyx_anneal::GoodhartReport>,
    metrics: FixtureMetrics,
}

#[derive(Clone, Copy, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct FixtureMetrics {
    mutual_info_panel_anchor: f64,
    n_eff: f64,
    panel_sufficiency: f64,
    kernel_recall: f64,
    oracle_accuracy: f64,
    mistake_rate: f64,
    compression_yield: f64,
    coverage: f64,
    dpi_ceiling: f64,
    #[serde(default)]
    provisional_count: usize,
    #[serde(default)]
    generated_positive_credit: JGeneratedPositiveCredit,
    #[serde(default)]
    synthetic_recursion_credit_attempted: bool,
}

impl JMetricSources for FixtureMetrics {
    fn mutual_info_panel_anchor(&self) -> f64 {
        self.mutual_info_panel_anchor
    }

    fn n_eff(&self) -> f64 {
        self.n_eff
    }

    fn panel_sufficiency(&self, _domain: &str) -> f64 {
        self.panel_sufficiency
    }

    fn kernel_recall(&self) -> f64 {
        self.kernel_recall
    }

    fn oracle_accuracy(&self) -> f64 {
        self.oracle_accuracy
    }

    fn mistake_rate(&self) -> f64 {
        self.mistake_rate
    }

    fn compression_yield(&self) -> f64 {
        self.compression_yield
    }

    fn coverage(&self) -> f64 {
        self.coverage
    }

    fn dpi_ceiling(&self) -> f64 {
        self.dpi_ceiling
    }

    fn provisional_count(&self) -> usize {
        self.provisional_count
    }

    fn generated_positive_credit(&self) -> JGeneratedPositiveCredit {
        self.generated_positive_credit
    }

    fn synthetic_recursion_credit_attempted(&self) -> bool {
        self.synthetic_recursion_credit_attempted
    }
}

fn unavailable_j_value(weights: JWeights, goodhart_penalty: f64) -> JValue {
    JValue {
        j: f64::NAN,
        terms: JTerms {
            w1_info: f64::NAN,
            w2_n_eff: f64::NAN,
            w3_sufficiency: f64::NAN,
            w4_kernel_recall: f64::NAN,
            w5_oracle_accuracy: f64::NAN,
            w6_mistake_rate: f64::NAN,
            w7_compression: f64::NAN,
            w8_coverage: f64::NAN,
            p_redundant: f64::NAN,
            p_ungrounded: f64::NAN,
            p_goodhart: goodhart_penalty,
        },
        dpi_ceiling: f64::NAN,
        dpi_headroom: f64::NAN,
        provisional_excluded: 0,
        weights,
    }
}
