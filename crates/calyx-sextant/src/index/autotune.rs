//! Anneal-facing beamwidth/posting-cutoff autotune hook.

use std::collections::VecDeque;

use calyx_anneal::{BanditPolicy, ConfigBandit};
use calyx_core::{Clock, Ts};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::error::CALYX_ANNEAL_UNAVAILABLE;

pub const DEFAULT_TUNER_WINDOW: usize = 512;
pub const DEFAULT_RECALL_FLOOR: f32 = 0.85;
pub const DEFAULT_HYSTERESIS_WINDOW: u64 = 50;
pub const DEFAULT_LATENCY_SLO_US: u64 = 25_000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TunerObservation {
    pub query_latency_us: u64,
    pub recall_at_10: f32,
    pub beamwidth: usize,
    pub posting_cutoff: usize,
}

impl TunerObservation {
    pub fn from_clock(
        clock: &dyn Clock,
        started_at_ms: Ts,
        recall_at_10: f32,
        beamwidth: usize,
        posting_cutoff: usize,
    ) -> Self {
        Self {
            query_latency_us: clock
                .now()
                .saturating_sub(started_at_ms)
                .saturating_mul(1000),
            recall_at_10,
            beamwidth,
            posting_cutoff,
        }
    }

    fn params(self) -> BwPostcutoffConfig {
        BwPostcutoffConfig {
            beamwidth: self.beamwidth,
            posting_cutoff: self.posting_cutoff,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BwPostcutoffConfig {
    pub beamwidth: usize,
    pub posting_cutoff: usize,
}

impl Default for BwPostcutoffConfig {
    fn default() -> Self {
        Self {
            beamwidth: 64,
            posting_cutoff: 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TunerRange {
    pub min: usize,
    pub max: usize,
    pub step: usize,
}

impl TunerRange {
    pub const fn new(min: usize, max: usize, step: usize) -> Self {
        Self { min, max, step }
    }

    pub fn clamp(self, value: usize) -> usize {
        value.clamp(self.min, self.max)
    }

    fn up(self, value: usize) -> usize {
        self.clamp(value.saturating_add(self.step))
    }

    fn down(self, value: usize) -> usize {
        self.clamp(value.saturating_sub(self.step))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TunerConfig {
    pub beamwidth: TunerRange,
    pub posting_cutoff: TunerRange,
    pub window: usize,
    pub hysteresis_window: u64,
    pub latency_slo_us: u64,
    pub recall_floor: f32,
}

impl Default for TunerConfig {
    fn default() -> Self {
        Self {
            beamwidth: TunerRange::new(8, 512, 8),
            posting_cutoff: TunerRange::new(64, 65_536, 64),
            window: DEFAULT_TUNER_WINDOW,
            hysteresis_window: DEFAULT_HYSTERESIS_WINDOW,
            latency_slo_us: DEFAULT_LATENCY_SLO_US,
            recall_floor: DEFAULT_RECALL_FLOOR,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TuneDirection {
    BeamwidthDown,
    BeamwidthUp,
    PostingCutoffDown,
    PostingCutoffUp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunerAdjustmentKind {
    Proposal,
    Revert,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TunerLedgerEntry {
    pub event: String,
    pub reason: String,
    pub old_bw: usize,
    pub new_bw: usize,
    pub old_posting_cutoff: usize,
    pub new_posting_cutoff: usize,
    pub recall_observed: f32,
    pub p99_latency_us: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TunerAdjustment {
    pub kind: TunerAdjustmentKind,
    pub old: BwPostcutoffConfig,
    pub new: BwPostcutoffConfig,
    pub direction: Option<TuneDirection>,
    pub recall_observed: f32,
    pub p99_latency_us: u64,
    pub ledger_entry: Option<TunerLedgerEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TunerWarning {
    pub code: &'static str,
    pub message: String,
}

#[derive(Clone, Copy)]
struct WindowStats {
    p99_latency_us: u64,
    min_recall: f32,
}

pub trait BwPostcutoffAnnealRegistry {
    fn register_bw_postcutoff(&mut self, tuner: &BwPostcutoffTuner) -> bool;
}

pub struct BwPostcutoffTuner {
    config: TunerConfig,
    window: VecDeque<TunerObservation>,
    bandit: ConfigBandit,
    current: BwPostcutoffConfig,
    stable: BwPostcutoffConfig,
    observations_seen: u64,
    last_direction: Option<TuneDirection>,
    last_direction_at: u64,
    ledger: Vec<TunerLedgerEntry>,
    adjustments: Vec<TunerAdjustment>,
    warnings: Vec<TunerWarning>,
    standalone: bool,
}

impl BwPostcutoffTuner {
    pub fn new(initial: BwPostcutoffConfig) -> Self {
        Self::with_config(initial, TunerConfig::default())
    }

    pub fn with_config(initial: BwPostcutoffConfig, config: TunerConfig) -> Self {
        let initial = clamp_config(initial, config);
        let mut out = Self {
            config,
            window: VecDeque::new(),
            bandit: ConfigBandit::new(BanditPolicy::EpsilonGreedy { epsilon: 0.0 }, 550)
                .with_hysteresis(1),
            current: initial,
            stable: initial,
            observations_seen: 0,
            last_direction: None,
            last_direction_at: 0,
            ledger: Vec::new(),
            adjustments: Vec::new(),
            warnings: Vec::new(),
            standalone: false,
        };
        out.rebuild_bandit(initial);
        out
    }

    pub fn observe(&mut self, obs: TunerObservation) {
        self.observations_seen = self.observations_seen.saturating_add(1);
        self.current = clamp_config(obs.params(), self.config);
        if obs.recall_at_10 >= self.config.recall_floor {
            self.stable = self.current;
        }
        self.window.push_back(obs);
        while self.window.len() > self.config.window {
            self.window.pop_front();
        }
    }

    pub fn maybe_adjust(&mut self) -> Option<TunerAdjustment> {
        let stats = self.stats()?;
        if stats.min_recall < self.config.recall_floor {
            let adjustment = self.revert(stats);
            return Some(self.record_adjustment(adjustment));
        }
        if self.window.len() < self.config.window
            || stats.p99_latency_us <= self.config.latency_slo_us
        {
            return None;
        }
        let (candidate, direction, arm_idx) = self.best_latency_candidate()?;
        if candidate == self.current || self.hysteresis_blocks(direction) {
            return None;
        }
        let _ = self.bandit.record_result(arm_idx, true);
        self.last_direction = Some(direction);
        self.last_direction_at = self.observations_seen;
        let entry = TunerLedgerEntry {
            event: "diskann_tuner_adjust".to_string(),
            reason: "latency_above_slo".to_string(),
            old_bw: self.current.beamwidth,
            new_bw: candidate.beamwidth,
            old_posting_cutoff: self.current.posting_cutoff,
            new_posting_cutoff: candidate.posting_cutoff,
            recall_observed: stats.min_recall,
            p99_latency_us: stats.p99_latency_us,
        };
        self.ledger.push(entry.clone());
        Some(self.record_adjustment(TunerAdjustment {
            kind: TunerAdjustmentKind::Proposal,
            old: self.current,
            new: candidate,
            direction: Some(direction),
            recall_observed: stats.min_recall,
            p99_latency_us: stats.p99_latency_us,
            ledger_entry: Some(entry),
        }))
    }

    pub fn preview_direction(&self, direction: TuneDirection) -> BwPostcutoffConfig {
        candidate_for(self.current, self.config, direction)
    }

    pub fn ledger_entries(&self) -> &[TunerLedgerEntry] {
        &self.ledger
    }

    pub fn adjustment_history(&self) -> &[TunerAdjustment] {
        &self.adjustments
    }

    pub fn warnings(&self) -> &[TunerWarning] {
        &self.warnings
    }

    pub fn is_standalone(&self) -> bool {
        self.standalone
    }

    pub fn current_config(&self) -> BwPostcutoffConfig {
        self.current
    }

    fn stats(&self) -> Option<WindowStats> {
        if self.window.is_empty() {
            return None;
        }
        let mut latencies = self
            .window
            .iter()
            .map(|obs| obs.query_latency_us)
            .collect::<Vec<_>>();
        latencies.sort_unstable();
        let p99_idx = (latencies.len() * 99).div_ceil(100).saturating_sub(1);
        let min_recall = self
            .window
            .iter()
            .map(|obs| obs.recall_at_10)
            .fold(f32::INFINITY, f32::min);
        Some(WindowStats {
            p99_latency_us: latencies[p99_idx],
            min_recall,
        })
    }

    fn best_latency_candidate(&mut self) -> Option<(BwPostcutoffConfig, TuneDirection, usize)> {
        self.rebuild_bandit(self.current);
        let directions = [
            TuneDirection::BeamwidthDown,
            TuneDirection::PostingCutoffDown,
            TuneDirection::BeamwidthUp,
            TuneDirection::PostingCutoffUp,
        ];
        directions
            .into_iter()
            .enumerate()
            .map(|(idx, direction)| {
                (
                    candidate_for(self.current, self.config, direction),
                    direction,
                    idx + 1,
                )
            })
            .find(|(candidate, _, _)| *candidate != self.current)
    }

    fn hysteresis_blocks(&self, direction: TuneDirection) -> bool {
        self.last_direction.is_some_and(|last| last != direction)
            && self
                .observations_seen
                .saturating_sub(self.last_direction_at)
                < self.config.hysteresis_window
    }

    fn revert(&mut self, stats: WindowStats) -> TunerAdjustment {
        let entry = TunerLedgerEntry {
            event: "diskann_tuner_revert".to_string(),
            reason: "recall_below_floor".to_string(),
            old_bw: self.current.beamwidth,
            new_bw: self.stable.beamwidth,
            old_posting_cutoff: self.current.posting_cutoff,
            new_posting_cutoff: self.stable.posting_cutoff,
            recall_observed: stats.min_recall,
            p99_latency_us: stats.p99_latency_us,
        };
        self.current = self.stable;
        self.ledger.push(entry.clone());
        TunerAdjustment {
            kind: TunerAdjustmentKind::Revert,
            old: BwPostcutoffConfig {
                beamwidth: entry.old_bw,
                posting_cutoff: entry.old_posting_cutoff,
            },
            new: self.stable,
            direction: None,
            recall_observed: stats.min_recall,
            p99_latency_us: stats.p99_latency_us,
            ledger_entry: Some(entry),
        }
    }

    fn record_adjustment(&mut self, adjustment: TunerAdjustment) -> TunerAdjustment {
        self.adjustments.push(adjustment.clone());
        adjustment
    }

    fn rebuild_bandit(&mut self, current: BwPostcutoffConfig) {
        let mut bandit =
            ConfigBandit::new(BanditPolicy::EpsilonGreedy { epsilon: 0.0 }, 550).with_hysteresis(1);
        bandit.add_arm(config_bytes(current));
        for direction in [
            TuneDirection::BeamwidthDown,
            TuneDirection::PostingCutoffDown,
            TuneDirection::BeamwidthUp,
            TuneDirection::PostingCutoffUp,
        ] {
            bandit.add_arm(config_bytes(candidate_for(current, self.config, direction)));
        }
        self.bandit = bandit;
    }

    fn warn_standalone(&mut self, message: impl Into<String>) {
        let message = message.into();
        warn!(code = CALYX_ANNEAL_UNAVAILABLE, "{message}");
        self.standalone = true;
        self.warnings.push(TunerWarning {
            code: CALYX_ANNEAL_UNAVAILABLE,
            message,
        });
    }
}

pub fn register_with_anneal<R>(
    mut tuner: BwPostcutoffTuner,
    anneal: Option<&mut R>,
) -> BwPostcutoffTuner
where
    R: BwPostcutoffAnnealRegistry,
{
    match anneal {
        Some(registry) => {
            if registry.register_bw_postcutoff(&tuner) {
                tuner
            } else {
                tuner.warn_standalone("Anneal registry rejected bw_postcutoff observer");
                tuner
            }
        }
        None => {
            tuner.warn_standalone("Anneal registry unavailable for bw_postcutoff observer");
            tuner
        }
    }
}

fn candidate_for(
    current: BwPostcutoffConfig,
    config: TunerConfig,
    direction: TuneDirection,
) -> BwPostcutoffConfig {
    match direction {
        TuneDirection::BeamwidthDown => BwPostcutoffConfig {
            beamwidth: config.beamwidth.down(current.beamwidth),
            ..current
        },
        TuneDirection::BeamwidthUp => BwPostcutoffConfig {
            beamwidth: config.beamwidth.up(current.beamwidth),
            ..current
        },
        TuneDirection::PostingCutoffDown => BwPostcutoffConfig {
            posting_cutoff: config.posting_cutoff.down(current.posting_cutoff),
            ..current
        },
        TuneDirection::PostingCutoffUp => BwPostcutoffConfig {
            posting_cutoff: config.posting_cutoff.up(current.posting_cutoff),
            ..current
        },
    }
}

fn clamp_config(config: BwPostcutoffConfig, ranges: TunerConfig) -> BwPostcutoffConfig {
    BwPostcutoffConfig {
        beamwidth: ranges.beamwidth.clamp(config.beamwidth),
        posting_cutoff: ranges.posting_cutoff.clamp(config.posting_cutoff),
    }
}

fn config_bytes(config: BwPostcutoffConfig) -> Vec<u8> {
    serde_json::to_vec(&config).expect("BwPostcutoffConfig serializes")
}
