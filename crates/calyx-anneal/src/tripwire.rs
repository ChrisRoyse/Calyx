use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::Result;
use serde::{Deserialize, Serialize};

mod errors;
mod persistence;

use errors::{invalid_config, invalid_metric};
use persistence::atomic_write_text;

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
        let (thresholds, state) = if config_path.exists() {
            read_registry(&config_path)?
        } else {
            let thresholds = default_thresholds();
            let state = initial_states(&thresholds);
            persist_registry(&config_path, &thresholds, &state)?;
            (thresholds, state)
        };
        Ok(Self {
            config_path,
            thresholds,
            state,
        })
    }

    pub fn check(&mut self, metric: TripwireMetric, value: f64) -> Result<TripwireResult> {
        if !value.is_finite() {
            return Err(invalid_metric(metric, value));
        }
        let threshold = *self
            .thresholds
            .get(&metric)
            .ok_or_else(|| invalid_config(format!("missing threshold for {}", metric.key())))?;
        let mut next = self
            .state
            .get(&metric)
            .copied()
            .unwrap_or_else(|| initial_state(threshold));
        next.last_value = value;
        next.crossed = threshold_crossed(threshold, next.crossed, value);
        let mut candidate_state = self.state.clone();
        candidate_state.insert(metric, next);
        persist_registry(&self.config_path, &self.thresholds, &candidate_state)?;
        self.state = candidate_state;
        Ok(if next.crossed {
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
        let mut candidate_state = self.state.clone();
        let prior = candidate_state
            .get(&metric)
            .copied()
            .unwrap_or_else(|| initial_state(threshold));
        candidate_state.insert(
            metric,
            ThresholdState {
                last_value: prior.last_value,
                crossed: threshold_crossed(threshold, prior.crossed, prior.last_value),
            },
        );
        persist_registry(&self.config_path, &candidate, &candidate_state)?;
        self.thresholds = candidate;
        self.state = candidate_state;
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
}

pub fn tripwire_config_path(vault: &Path) -> PathBuf {
    vault.join(CONFIG_DIR).join(CONFIG_FILE)
}

pub fn read_tripwire_config_from_vault(vault: impl AsRef<Path>) -> Result<TripwireConfigReadback> {
    let config_path = tripwire_config_path(vault.as_ref());
    let (thresholds, _) = read_registry(&config_path)?;
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

fn read_registry(
    path: &Path,
) -> Result<(
    HashMap<TripwireMetric, TripwireThreshold>,
    HashMap<TripwireMetric, ThresholdState>,
)> {
    let bytes = fs::read(path)
        .map_err(|error| invalid_config(format!("read {}: {error}", path.display())))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| invalid_config(format!("{} is not UTF-8: {error}", path.display())))?;
    let file: TripwireFile = toml::from_str(text)
        .map_err(|error| invalid_config(format!("parse {}: {error}", path.display())))?;
    file.into_registry(path)
}

fn persist_registry(
    path: &Path,
    thresholds: &HashMap<TripwireMetric, TripwireThreshold>,
    state: &HashMap<TripwireMetric, ThresholdState>,
) -> Result<()> {
    let file = TripwireFile::from_registry(thresholds, state)?;
    let text = toml::to_string_pretty(&file)
        .map_err(|error| invalid_config(format!("serialize tripwire config: {error}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| invalid_config(format!("create {}: {error}", parent.display())))?;
    }
    atomic_write_text(path, &text)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TripwireFile {
    thresholds: BTreeMap<String, TripwireThreshold>,
    #[serde(default)]
    state: BTreeMap<String, ThresholdState>,
}

impl TripwireFile {
    fn from_registry(
        thresholds: &HashMap<TripwireMetric, TripwireThreshold>,
        state: &HashMap<TripwireMetric, ThresholdState>,
    ) -> Result<Self> {
        ensure_all_metrics_present(thresholds)?;
        let mut persisted_thresholds = BTreeMap::new();
        let mut persisted_state = BTreeMap::new();
        for metric in METRICS {
            let threshold = *thresholds
                .get(&metric)
                .ok_or_else(|| invalid_config(format!("missing threshold for {}", metric.key())))?;
            validate_threshold(metric, threshold)?;
            let state = state
                .get(&metric)
                .copied()
                .unwrap_or_else(|| initial_state(threshold));
            validate_state(metric, threshold, state)?;
            persisted_thresholds.insert(metric.key().to_string(), threshold);
            persisted_state.insert(metric.key().to_string(), state);
        }
        Ok(Self {
            thresholds: persisted_thresholds,
            state: persisted_state,
        })
    }

    fn into_registry(
        self,
        path: &Path,
    ) -> Result<(
        HashMap<TripwireMetric, TripwireThreshold>,
        HashMap<TripwireMetric, ThresholdState>,
    )> {
        let mut thresholds = HashMap::new();
        for (key, threshold) in self.thresholds {
            let metric = TripwireMetric::from_key(&key).ok_or_else(|| {
                invalid_config(format!("{} contains unknown metric {key}", path.display()))
            })?;
            validate_threshold(metric, threshold)?;
            thresholds.insert(metric, threshold);
        }
        ensure_all_metrics_present(&thresholds)?;
        let mut state = initial_states(&thresholds);
        for (key, persisted) in self.state {
            let metric = TripwireMetric::from_key(&key).ok_or_else(|| {
                invalid_config(format!(
                    "{} contains unknown state metric {key}",
                    path.display()
                ))
            })?;
            let threshold = *thresholds
                .get(&metric)
                .ok_or_else(|| invalid_config(format!("missing threshold for {key}")))?;
            validate_state(metric, threshold, persisted)?;
            state.insert(metric, persisted);
        }
        Ok((thresholds, state))
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

fn initial_states(
    thresholds: &HashMap<TripwireMetric, TripwireThreshold>,
) -> HashMap<TripwireMetric, ThresholdState> {
    thresholds
        .iter()
        .map(|(metric, threshold)| (*metric, initial_state(*threshold)))
        .collect()
}

fn validate_state(
    metric: TripwireMetric,
    threshold: TripwireThreshold,
    state: ThresholdState,
) -> Result<()> {
    if !state.last_value.is_finite() {
        return Err(invalid_config(format!(
            "{} state last_value must be finite",
            metric.key()
        )));
    }
    if threshold_crossed(threshold, state.crossed, state.last_value) != state.crossed {
        return Err(invalid_config(format!(
            "{} persisted hysteresis state contradicts last_value",
            metric.key()
        )));
    }
    Ok(())
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
