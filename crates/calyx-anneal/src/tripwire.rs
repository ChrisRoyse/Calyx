use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

pub const CALYX_TRIPWIRE_INVALID_METRIC: &str = "CALYX_TRIPWIRE_INVALID_METRIC";
pub const CALYX_TRIPWIRE_INVALID_CONFIG: &str = "CALYX_TRIPWIRE_INVALID_CONFIG";

const CONFIG_DIR: &str = ".anneal";
const CONFIG_FILE: &str = "tripwire.toml";
const DEFAULT_HYSTERESIS_FRACTION: f64 = 0.05;
const TRIPWIRE_EPSILON: f64 = 1e-12;

const METRICS: [TripwireMetric; 5] = [
    TripwireMetric::RecallAtK,
    TripwireMetric::GuardFAR,
    TripwireMetric::GuardFRR,
    TripwireMetric::SearchP99,
    TripwireMetric::IngestP95,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TripwireMetric {
    #[serde(rename = "recall_at_k")]
    RecallAtK,
    #[serde(rename = "guard_far")]
    GuardFAR,
    #[serde(rename = "guard_frr")]
    GuardFRR,
    #[serde(rename = "search_p99")]
    SearchP99,
    #[serde(rename = "ingest_p95")]
    IngestP95,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdDir {
    Below,
    Above,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TripwireThreshold {
    pub bound: f64,
    pub hysteresis: f64,
    pub direction: ThresholdDir,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThresholdState {
    pub last_value: f64,
    pub crossed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TripwireStatus {
    pub metric: TripwireMetric,
    pub threshold: TripwireThreshold,
    pub state: ThresholdState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum TripwireResult {
    Ok,
    Crossed {
        metric: TripwireMetric,
        threshold: f64,
        hysteresis: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TripwireThresholdEntry {
    pub metric: TripwireMetric,
    pub threshold: TripwireThreshold,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TripwireConfigReadback {
    pub config_path: PathBuf,
    pub thresholds: Vec<TripwireThresholdEntry>,
}

#[derive(Clone, Debug)]
pub struct TripwireRegistry {
    config_path: PathBuf,
    thresholds: HashMap<TripwireMetric, TripwireThreshold>,
    state: HashMap<TripwireMetric, ThresholdState>,
}

impl TripwireRegistry {
    pub fn load_from_vault(vault: impl AsRef<Path>) -> Result<Self> {
        let config_path = tripwire_config_path(vault.as_ref());
        let thresholds = if config_path.exists() {
            read_thresholds(&config_path)?
        } else {
            let defaults = default_thresholds();
            persist_thresholds(&config_path, &defaults)?;
            defaults
        };
        Ok(Self::from_thresholds(config_path, thresholds))
    }

    pub fn check(&mut self, metric: TripwireMetric, value: f64) -> Result<TripwireResult> {
        if !value.is_finite() {
            return Err(invalid_metric(metric, value));
        }
        let threshold = *self
            .thresholds
            .get(&metric)
            .ok_or_else(|| invalid_config(format!("missing threshold for {}", metric.key())))?;
        let state = self
            .state
            .entry(metric)
            .or_insert_with(|| initial_state(threshold));
        state.last_value = value;
        state.crossed = threshold_crossed(threshold, state.crossed, value);
        Ok(if state.crossed {
            TripwireResult::Crossed {
                metric,
                threshold: threshold.bound,
                hysteresis: threshold.hysteresis,
            }
        } else {
            TripwireResult::Ok
        })
    }

    pub fn set_tripwire(
        &mut self,
        metric: TripwireMetric,
        bound: f64,
        hysteresis: f64,
    ) -> Result<()> {
        let threshold = TripwireThreshold {
            bound,
            hysteresis,
            direction: default_direction(metric),
        };
        validate_threshold(metric, threshold)?;
        let mut candidate = self.thresholds.clone();
        candidate.insert(metric, threshold);
        ensure_all_metrics_present(&candidate)?;
        persist_thresholds(&self.config_path, &candidate)?;
        self.thresholds = candidate;
        self.state
            .entry(metric)
            .or_insert_with(|| initial_state(threshold));
        Ok(())
    }

    pub fn status(&self) -> Vec<TripwireStatus> {
        METRICS
            .iter()
            .copied()
            .filter_map(|metric| {
                let threshold = *self.thresholds.get(&metric)?;
                let state = *self.state.get(&metric).unwrap_or(&initial_state(threshold));
                Some(TripwireStatus {
                    metric,
                    threshold,
                    state,
                })
            })
            .collect()
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    fn from_thresholds(
        config_path: PathBuf,
        thresholds: HashMap<TripwireMetric, TripwireThreshold>,
    ) -> Self {
        let state = thresholds
            .iter()
            .map(|(metric, threshold)| (*metric, initial_state(*threshold)))
            .collect();
        Self {
            config_path,
            thresholds,
            state,
        }
    }
}

pub fn tripwire_config_path(vault: &Path) -> PathBuf {
    vault.join(CONFIG_DIR).join(CONFIG_FILE)
}

pub fn read_tripwire_config_from_vault(vault: impl AsRef<Path>) -> Result<TripwireConfigReadback> {
    let config_path = tripwire_config_path(vault.as_ref());
    let thresholds = read_thresholds(&config_path)?;
    Ok(TripwireConfigReadback {
        config_path,
        thresholds: threshold_entries(&thresholds),
    })
}

fn default_thresholds() -> HashMap<TripwireMetric, TripwireThreshold> {
    METRICS
        .iter()
        .copied()
        .map(|metric| {
            let bound = default_bound(metric);
            (
                metric,
                TripwireThreshold {
                    bound,
                    hysteresis: bound * DEFAULT_HYSTERESIS_FRACTION,
                    direction: default_direction(metric),
                },
            )
        })
        .collect()
}

fn read_thresholds(path: &Path) -> Result<HashMap<TripwireMetric, TripwireThreshold>> {
    let bytes = fs::read(path)
        .map_err(|error| invalid_config(format!("read {}: {error}", path.display())))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| invalid_config(format!("{} is not UTF-8: {error}", path.display())))?;
    let file: TripwireFile = toml::from_str(text)
        .map_err(|error| invalid_config(format!("parse {}: {error}", path.display())))?;
    file.into_thresholds(path)
}

fn persist_thresholds(
    path: &Path,
    thresholds: &HashMap<TripwireMetric, TripwireThreshold>,
) -> Result<()> {
    let file = TripwireFile::from_thresholds(thresholds)?;
    let text = toml::to_string_pretty(&file)
        .map_err(|error| invalid_config(format!("serialize tripwire config: {error}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| invalid_config(format!("create {}: {error}", parent.display())))?;
    }
    fs::write(path, text)
        .map_err(|error| invalid_config(format!("write {}: {error}", path.display())))?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TripwireFile {
    thresholds: BTreeMap<String, TripwireThreshold>,
}

impl TripwireFile {
    fn from_thresholds(thresholds: &HashMap<TripwireMetric, TripwireThreshold>) -> Result<Self> {
        ensure_all_metrics_present(thresholds)?;
        let mut persisted = BTreeMap::new();
        for metric in METRICS {
            let threshold = *thresholds
                .get(&metric)
                .ok_or_else(|| invalid_config(format!("missing threshold for {}", metric.key())))?;
            validate_threshold(metric, threshold)?;
            persisted.insert(metric.key().to_string(), threshold);
        }
        Ok(Self {
            thresholds: persisted,
        })
    }

    fn into_thresholds(self, path: &Path) -> Result<HashMap<TripwireMetric, TripwireThreshold>> {
        let mut thresholds = HashMap::new();
        for (key, threshold) in self.thresholds {
            let metric = TripwireMetric::from_key(&key).ok_or_else(|| {
                invalid_config(format!("{} contains unknown metric {key}", path.display()))
            })?;
            validate_threshold(metric, threshold)?;
            thresholds.insert(metric, threshold);
        }
        ensure_all_metrics_present(&thresholds)?;
        Ok(thresholds)
    }
}

fn threshold_entries(
    thresholds: &HashMap<TripwireMetric, TripwireThreshold>,
) -> Vec<TripwireThresholdEntry> {
    METRICS
        .iter()
        .copied()
        .filter_map(|metric| {
            thresholds
                .get(&metric)
                .map(|threshold| TripwireThresholdEntry {
                    metric,
                    threshold: *threshold,
                })
        })
        .collect()
}

fn ensure_all_metrics_present(
    thresholds: &HashMap<TripwireMetric, TripwireThreshold>,
) -> Result<()> {
    for metric in METRICS {
        if !thresholds.contains_key(&metric) {
            return Err(invalid_config(format!(
                "tripwire config missing {}",
                metric.key()
            )));
        }
    }
    Ok(())
}

fn validate_threshold(metric: TripwireMetric, threshold: TripwireThreshold) -> Result<()> {
    if !threshold.bound.is_finite() || threshold.bound < 0.0 {
        return Err(invalid_config(format!(
            "{} bound must be finite and non-negative",
            metric.key()
        )));
    }
    if !threshold.hysteresis.is_finite() || threshold.hysteresis < 0.0 {
        return Err(invalid_config(format!(
            "{} hysteresis must be finite and non-negative",
            metric.key()
        )));
    }
    if threshold.direction != default_direction(metric) {
        return Err(invalid_config(format!(
            "{} direction must be {:?}",
            metric.key(),
            default_direction(metric)
        )));
    }
    if threshold.direction == ThresholdDir::Below && threshold.hysteresis > threshold.bound {
        return Err(invalid_config(format!(
            "{} lower-bound hysteresis exceeds bound",
            metric.key()
        )));
    }
    Ok(())
}

fn threshold_crossed(threshold: TripwireThreshold, was_crossed: bool, value: f64) -> bool {
    match (threshold.direction, was_crossed) {
        (ThresholdDir::Below, false) => value < threshold.bound - TRIPWIRE_EPSILON,
        (ThresholdDir::Below, true) => {
            value < threshold.bound + threshold.hysteresis - TRIPWIRE_EPSILON
        }
        (ThresholdDir::Above, false) => value > threshold.bound + TRIPWIRE_EPSILON,
        (ThresholdDir::Above, true) => {
            value > threshold.bound - threshold.hysteresis + TRIPWIRE_EPSILON
        }
    }
}

fn initial_state(threshold: TripwireThreshold) -> ThresholdState {
    ThresholdState {
        last_value: threshold.bound,
        crossed: false,
    }
}

fn default_bound(metric: TripwireMetric) -> f64 {
    match metric {
        TripwireMetric::RecallAtK => 0.90,
        TripwireMetric::GuardFAR => 0.01,
        TripwireMetric::GuardFRR => 0.05,
        TripwireMetric::SearchP99 => 200.0,
        TripwireMetric::IngestP95 => 500.0,
    }
}

fn default_direction(metric: TripwireMetric) -> ThresholdDir {
    match metric {
        TripwireMetric::RecallAtK => ThresholdDir::Below,
        TripwireMetric::GuardFAR
        | TripwireMetric::GuardFRR
        | TripwireMetric::SearchP99
        | TripwireMetric::IngestP95 => ThresholdDir::Above,
    }
}

fn invalid_metric(metric: TripwireMetric, value: f64) -> CalyxError {
    CalyxError {
        code: CALYX_TRIPWIRE_INVALID_METRIC,
        message: format!("{} metric value must be finite, got {value}", metric.key()),
        remediation: "drop the Anneal candidate and re-measure the guarded metric",
    }
}

fn invalid_config(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_TRIPWIRE_INVALID_CONFIG,
        message: message.into(),
        remediation: "fix vault .anneal/tripwire.toml before running Anneal",
    }
}

impl TripwireMetric {
    fn key(self) -> &'static str {
        match self {
            Self::RecallAtK => "recall_at_k",
            Self::GuardFAR => "guard_far",
            Self::GuardFRR => "guard_frr",
            Self::SearchP99 => "search_p99",
            Self::IngestP95 => "ingest_p95",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        match key {
            "recall_at_k" => Some(Self::RecallAtK),
            "guard_far" => Some(Self::GuardFAR),
            "guard_frr" => Some(Self::GuardFRR),
            "search_p99" => Some(Self::SearchP99),
            "ingest_p95" => Some(Self::IngestP95),
            _ => None,
        }
    }
}
