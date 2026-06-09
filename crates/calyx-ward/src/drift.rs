//! Rolling drift monitoring for calibrated Ward guards.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

use calyx_core::SlotId;
use serde::{Deserialize, Serialize};

use crate::profile::{GuardId, GuardProfile};
use crate::verdict::GuardVerdict;

pub const DEFAULT_DRIFT_WINDOW: usize = 500;
pub const DEFAULT_DRIFT_CHANNEL_CAPACITY: usize = 32;
pub const REJECTION_RATE_DRIFT_MULTIPLIER: f32 = 1.5;

/// Event sent to Anneal when a slot's rolling rejection rate creeps upward.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DriftEvent {
    pub guard_id: GuardId,
    pub slot: SlotId,
    pub current_rejection_rate: f32,
    pub calibrated_far_bound: f32,
}

/// Snapshot returned by `guard_health()`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuardHealth {
    pub guard_id: GuardId,
    pub per_slot_rejection_rate: BTreeMap<SlotId, f32>,
    #[serde(default)]
    pub per_slot_calibrated_far_bound: BTreeMap<SlotId, f32>,
    pub per_slot_frr: BTreeMap<SlotId, f32>,
    pub drift: bool,
    pub last_calibrated: i64,
    pub dropped_events: usize,
}

/// Object-safe hook used until Anneal's PH48 queue is live.
pub trait AnnealHook: Send + Sync + 'static {
    fn on_rejection_rate_drift(
        &self,
        guard_id: GuardId,
        slot: SlotId,
        current_rejection_rate: f32,
        calibrated_far_bound: f32,
    );
}

/// Rolling drift monitor for one calibrated guard profile.
pub struct DriftMonitor {
    guard_id: GuardId,
    window_size: usize,
    per_slot_results: BTreeMap<SlotId, VecDeque<bool>>,
    calibrated_far_bound: BTreeMap<SlotId, f32>,
    calibrated_frr: BTreeMap<SlotId, f32>,
    drift_slots: BTreeSet<SlotId>,
    notified_drift_slots: BTreeSet<SlotId>,
    last_calibrated: i64,
    _anneal_hook: Arc<dyn AnnealHook>,
    hook_channel: Option<SyncSender<DriftEvent>>,
    worker: Option<JoinHandle<()>>,
    dropped_events: usize,
}

impl DriftMonitor {
    /// Builds a monitor from a guard profile with the default bounded channel.
    pub fn new(
        profile: &GuardProfile,
        window_size: usize,
        anneal_hook: Arc<dyn AnnealHook>,
    ) -> Self {
        Self::with_channel_capacity(
            profile,
            window_size,
            DEFAULT_DRIFT_CHANNEL_CAPACITY,
            anneal_hook,
        )
    }

    /// Builds a monitor with an injected channel capacity for tests and FSV.
    pub fn with_channel_capacity(
        profile: &GuardProfile,
        window_size: usize,
        channel_capacity: usize,
        anneal_hook: Arc<dyn AnnealHook>,
    ) -> Self {
        let (calibrated_far_bound, calibrated_frr, last_calibrated) = calibration_maps(profile);
        let (sender, receiver) = mpsc::sync_channel::<DriftEvent>(channel_capacity);
        let worker_hook = Arc::clone(&anneal_hook);
        let worker = thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                worker_hook.on_rejection_rate_drift(
                    event.guard_id,
                    event.slot,
                    event.current_rejection_rate,
                    event.calibrated_far_bound,
                );
            }
        });

        Self {
            guard_id: profile.guard_id,
            window_size: window_size.max(1),
            per_slot_results: BTreeMap::new(),
            calibrated_far_bound,
            calibrated_frr,
            drift_slots: BTreeSet::new(),
            notified_drift_slots: BTreeSet::new(),
            last_calibrated,
            _anneal_hook: anneal_hook,
            hook_channel: Some(sender),
            worker: Some(worker),
            dropped_events: 0,
        }
    }

    /// Records a Ward verdict without blocking the guard hot path.
    pub fn record_verdict(&mut self, verdict: &GuardVerdict) {
        if verdict.guard_id != self.guard_id {
            return;
        }

        for slot_verdict in &verdict.per_slot {
            let window = self.per_slot_results.entry(slot_verdict.slot).or_default();
            window.push_back(slot_verdict.pass);
            while window.len() > self.window_size {
                window.pop_front();
            }
            self.check_slot(slot_verdict.slot);
        }
    }

    pub fn dropped_events(&self) -> usize {
        self.dropped_events
    }

    fn check_slot(&mut self, slot: SlotId) {
        let Some(calibrated_far_bound) = self.calibrated_far_bound.get(&slot).copied() else {
            return;
        };
        let current_rejection_rate = self.rolling_rejection_rate(slot);
        let drift = current_rejection_rate > calibrated_far_bound * REJECTION_RATE_DRIFT_MULTIPLIER;

        if drift {
            self.drift_slots.insert(slot);
            if self.notified_drift_slots.contains(&slot) {
                return;
            }
            let event = DriftEvent {
                guard_id: self.guard_id,
                slot,
                current_rejection_rate,
                calibrated_far_bound,
            };
            if let Some(sender) = &self.hook_channel {
                match sender.try_send(event) {
                    Ok(()) => {
                        self.notified_drift_slots.insert(slot);
                    }
                    Err(TrySendError::Full(_)) => self.dropped_events += 1,
                    Err(TrySendError::Disconnected(_)) => self.dropped_events += 1,
                }
            }
        } else if !drift {
            self.drift_slots.remove(&slot);
            self.notified_drift_slots.remove(&slot);
        }
    }

    fn rolling_rejection_rate(&self, slot: SlotId) -> f32 {
        self.per_slot_results
            .get(&slot)
            .map(rejection_fraction)
            .unwrap_or(0.0)
    }

    fn health(&self) -> GuardHealth {
        let mut per_slot_rejection_rate = BTreeMap::new();
        for slot in self.calibrated_far_bound.keys() {
            per_slot_rejection_rate.insert(*slot, self.rolling_rejection_rate(*slot));
        }
        for slot in self.per_slot_results.keys() {
            per_slot_rejection_rate
                .entry(*slot)
                .or_insert_with(|| self.rolling_rejection_rate(*slot));
        }

        GuardHealth {
            guard_id: self.guard_id,
            per_slot_rejection_rate,
            per_slot_calibrated_far_bound: self.calibrated_far_bound.clone(),
            per_slot_frr: self.calibrated_frr.clone(),
            drift: !self.drift_slots.is_empty(),
            last_calibrated: self.last_calibrated,
            dropped_events: self.dropped_events,
        }
    }
}

impl Drop for DriftMonitor {
    fn drop(&mut self) {
        self.hook_channel.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Returns a health snapshot for `guard_id`; unknown guards return zeros.
pub fn guard_health(monitor: &DriftMonitor, guard_id: GuardId) -> GuardHealth {
    if monitor.guard_id == guard_id {
        monitor.health()
    } else {
        GuardHealth {
            guard_id,
            per_slot_rejection_rate: BTreeMap::new(),
            per_slot_calibrated_far_bound: BTreeMap::new(),
            per_slot_frr: BTreeMap::new(),
            drift: false,
            last_calibrated: 0,
            dropped_events: 0,
        }
    }
}

fn calibration_maps(profile: &GuardProfile) -> (BTreeMap<SlotId, f32>, BTreeMap<SlotId, f32>, i64) {
    let profile_meta = profile.calibration.as_ref();
    let far = profile_meta.map_or(0.0, |meta| meta.far);
    let frr = profile_meta.map_or(0.0, |meta| meta.frr);
    let last_calibrated = profile_meta.map_or(0, |meta| meta.ts);
    let mut calibrated_far_bound = BTreeMap::new();
    let mut calibrated_frr = BTreeMap::new();
    for slot in profile.tau.keys() {
        if let Some(slot_meta) = profile_meta.and_then(|meta| meta.per_slot.get(slot)) {
            calibrated_far_bound.insert(*slot, slot_meta.far);
            calibrated_frr.insert(*slot, slot_meta.frr);
        } else {
            calibrated_far_bound.insert(*slot, far);
            calibrated_frr.insert(*slot, frr);
        }
    }
    (calibrated_far_bound, calibrated_frr, last_calibrated)
}

fn rejection_fraction(window: &VecDeque<bool>) -> f32 {
    if window.is_empty() {
        0.0
    } else {
        let failures = window.iter().filter(|passed| !**passed).count();
        failures as f32 / window.len() as f32
    }
}
