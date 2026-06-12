use std::fs;
use std::path::PathBuf;

use calyx_anneal::{
    DEFAULT_J_DOMAIN, JMetricSources, JObjectiveContext, JWeights, compute_j,
    read_objective_weights_from_vault,
};
use serde::Deserialize;
use serde_json::json;

pub(crate) fn run(args: &[String]) -> Result<(), String> {
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
    let context = JObjectiveContext {
        domain: fixture
            .domain
            .unwrap_or_else(|| DEFAULT_J_DOMAIN.to_string()),
        panel_len: fixture.panel_len,
        weights,
    };
    let j_value = compute_j(&context, &fixture.metrics).map_err(|error| error.to_string())?;
    let readback = json!({
        "source_of_truth": "fixture JSON bytes read by calyx anneal intelligence-report",
        "fixture_path": request.fixture.display().to_string(),
        "fixture_len": fixture_bytes.len(),
        "fixture_blake3": blake3::hash(&fixture_bytes).to_hex().to_string(),
        "weights_source": weights_source,
        "context": context,
        "raw_metrics": fixture.metrics,
        "j_value": j_value,
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
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    #[serde(default)]
    domain: Option<String>,
    panel_len: usize,
    #[serde(default)]
    weights: Option<JWeights>,
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
}
