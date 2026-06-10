//! Novelty routing for failed Ward verdicts.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{FREQUENCY_SCALAR, read_series};
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, VaultStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::WardError;
use crate::guard::ProducedSlots;
use crate::profile::{GuardId, GuardProfile, NoveltyAction};
use crate::verdict::{GuardVerdict, SlotVerdict};

/// Stable identifier for a novelty record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NovelId(Uuid);

impl NovelId {
    /// Builds a novelty id from a UUID.
    pub const fn new(value: Uuid) -> Self {
        Self(value)
    }

    /// Returns the wrapped UUID.
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for NovelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// Durable lifecycle status for a failed guard output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoveltyStatus {
    AwaitingGrounding,
    Quarantined,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Domain {
    pub id: String,
    pub cx_ids: Vec<CxId>,
}

impl Domain {
    pub fn new(id: impl Into<String>, cx_ids: Vec<CxId>) -> Self {
        Self {
            id: id.into(),
            cx_ids,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SurpriseScore(f32);

impl SurpriseScore {
    pub fn new(value: f32) -> Result<Self, WardError> {
        if value.is_finite() && value >= 0.0 {
            Ok(Self(value))
        } else {
            Err(WardError::InvalidDomain {
                reason: format!("surprise score must be finite and non-negative, found {value}"),
            })
        }
    }

    pub const fn get(self) -> f32 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "signal")]
pub enum NoveltySignal {
    Recurring {
        frequency: u64,
        cadence_secs: f64,
    },
    NonRecurring,
    OverdueRecurrence {
        expected_t: EpochSecs,
        overdue_by_secs: u64,
    },
    Anomaly {
        surprise_bits: SurpriseScore,
    },
}

/// Durable record written when Ward routes a failed verdict.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoveltyRecord {
    pub novel_id: NovelId,
    pub guard_id: GuardId,
    pub produced_slots: ProducedSlots,
    pub failing_verdicts: Vec<SlotVerdict>,
    pub action_taken: NoveltyAction,
    pub ts: i64,
    pub status: NoveltyStatus,
}

/// Storage seam for Ward novelty records.
pub trait VaultSink: Send + Sync {
    fn write_novel(&self, record: &NoveltyRecord) -> Result<(), WardError>;
    fn novel_records(&self) -> Result<Vec<NoveltyRecord>, WardError>;
}

/// Routes failed guard verdicts into the configured novelty sink.
pub struct NoveltyHandler {
    vault: Arc<dyn VaultSink>,
    clock: Arc<dyn Clock>,
}

impl NoveltyHandler {
    /// Builds a novelty handler around an object-safe vault sink and clock.
    pub fn new(vault: Arc<dyn VaultSink>, clock: Arc<dyn Clock>) -> Self {
        Self { vault, clock }
    }

    /// Writes one novelty record for a failed verdict and returns the route result.
    pub fn handle(
        &self,
        profile: &GuardProfile,
        verdict: &GuardVerdict,
        produced: &ProducedSlots,
    ) -> Result<NoveltyRecord, WardError> {
        if verdict.guard_id != profile.guard_id {
            return Err(WardError::GuardIdMismatch {
                profile_guard_id: profile.guard_id,
                verdict_guard_id: verdict.guard_id,
            });
        }
        if verdict.overall_pass {
            return Err(WardError::NotAFailure {
                guard_id: verdict.guard_id,
            });
        }

        let status = match profile.novelty_action {
            NoveltyAction::NewRegion => NoveltyStatus::AwaitingGrounding,
            NoveltyAction::Quarantine => NoveltyStatus::Quarantined,
            NoveltyAction::RejectClosed => NoveltyStatus::Rejected,
        };
        let failing_verdicts: Vec<_> = verdict
            .per_slot
            .iter()
            .filter(|slot| !slot.pass)
            .cloned()
            .collect();
        let ts = clock_ts_i64(self.clock.as_ref());
        let record = NoveltyRecord {
            novel_id: derive_novel_id(profile, verdict, produced, ts),
            guard_id: profile.guard_id,
            produced_slots: produced.clone(),
            failing_verdicts,
            action_taken: profile.novelty_action.clone(),
            ts,
            status,
        };
        self.vault.write_novel(&record)?;

        if matches!(profile.novelty_action, NoveltyAction::RejectClosed) {
            Err(WardError::Ood {
                guard_id: profile.guard_id,
                failing: record.failing_verdicts.clone(),
            })
        } else {
            Ok(record)
        }
    }
}

pub fn classify_novelty<C>(
    cx_id: CxId,
    vault: &AsterVault<C>,
    clock: &dyn Clock,
) -> Result<NoveltySignal, WardError>
where
    C: Clock,
{
    let frequency = read_base_frequency(vault, cx_id)?;
    if frequency <= 1 {
        return Ok(NoveltySignal::NonRecurring);
    }

    let series = read_series(vault, cx_id).map_err(ward_runtime)?;
    let cadence_secs = series.cadence_secs.unwrap_or(0.0);
    if frequency >= 3
        && cadence_secs.is_finite()
        && cadence_secs > 0.0
        && let Some(last_t) = series
            .occurrences
            .iter()
            .map(|occurrence| occurrence.t_k)
            .max()
    {
        let now_secs = clock_now_secs(clock);
        let overdue_threshold = last_t.0 as f64 + (2.0 * cadence_secs);
        if now_secs as f64 > overdue_threshold {
            let expected_t = expected_epoch(last_t, cadence_secs)?;
            return Ok(NoveltySignal::OverdueRecurrence {
                expected_t,
                overdue_by_secs: now_secs.saturating_sub(expected_t.0).max(0) as u64,
            });
        }
    }

    Ok(NoveltySignal::Recurring {
        frequency,
        cadence_secs,
    })
}

pub fn surprise_bits<C>(
    cx_id: CxId,
    domain: &Domain,
    vault: &AsterVault<C>,
) -> Result<SurpriseScore, WardError>
where
    C: Clock,
{
    let total = total_domain_events(domain, vault)?;
    if total == 0 {
        return surprise_score_from_counts(0, 0);
    }
    let frequency = read_base_frequency(vault, cx_id)?;
    surprise_score_from_counts(frequency, total)
}

pub fn surprise_score_from_counts(
    frequency: u64,
    total_domain_events: u64,
) -> Result<SurpriseScore, WardError> {
    if total_domain_events == 0 {
        return SurpriseScore::new(0.0);
    }
    let effective_frequency = frequency.max(1) as f32;
    let p = (effective_frequency / total_domain_events as f32).clamp(f32::MIN_POSITIVE, 1.0);
    // INVARIANT: SurpriseScore is for retrieval anomaly only; MUST NOT modify stored bits.
    SurpriseScore::new(-p.ln() / 2.0_f32.ln())
}

pub fn overdue_recurrence_scan<C>(
    domain: &Domain,
    vault: &AsterVault<C>,
    clock: &dyn Clock,
) -> Result<Vec<(CxId, NoveltySignal)>, WardError>
where
    C: Clock,
{
    let mut overdue = Vec::new();
    for cx_id in unique_domain_ids(domain) {
        let signal = classify_novelty(cx_id, vault, clock)?;
        if matches!(signal, NoveltySignal::OverdueRecurrence { .. }) {
            overdue.push((cx_id, signal));
        }
    }
    Ok(overdue)
}

pub fn novelty_action_for_signal(signal: &NoveltySignal) -> Option<NoveltyAction> {
    match signal {
        NoveltySignal::Recurring { .. } => None,
        NoveltySignal::NonRecurring | NoveltySignal::OverdueRecurrence { .. } => {
            Some(NoveltyAction::NewRegion)
        }
        NoveltySignal::Anomaly { .. } => Some(NoveltyAction::Quarantine),
    }
}

/// Lists awaiting-grounding novelty records at or after `since_ts`.
pub fn novel_regions(
    vault: &dyn VaultSink,
    since_ts: Option<i64>,
) -> Result<Vec<NoveltyRecord>, WardError> {
    let since_ts = since_ts.unwrap_or(i64::MIN);
    Ok(vault
        .novel_records()?
        .into_iter()
        .filter(|record| record.status == NoveltyStatus::AwaitingGrounding && record.ts >= since_ts)
        .collect())
}

fn clock_ts_i64(clock: &dyn Clock) -> i64 {
    i64::try_from(clock.now()).unwrap_or(i64::MAX)
}

fn clock_now_secs(clock: &dyn Clock) -> i64 {
    i64::try_from(clock.now() / 1000).unwrap_or(i64::MAX)
}

fn total_domain_events<C>(domain: &Domain, vault: &AsterVault<C>) -> Result<u64, WardError>
where
    C: Clock,
{
    unique_domain_ids(domain)
        .into_iter()
        .try_fold(0_u64, |total, cx_id| {
            total
                .checked_add(read_base_frequency(vault, cx_id)?)
                .ok_or_else(|| WardError::InvalidDomain {
                    reason: format!("domain {} frequency total overflowed u64", domain.id),
                })
        })
}

fn read_base_frequency<C>(vault: &AsterVault<C>, cx_id: CxId) -> Result<u64, WardError>
where
    C: Clock,
{
    let cx = vault
        .get(cx_id, vault.snapshot())
        .map_err(|_| WardError::MissingFrequency {
            cx_id,
            detail: "base row missing",
        })?;
    let Some(value) = cx.scalars.get(FREQUENCY_SCALAR) else {
        return Err(WardError::MissingFrequency {
            cx_id,
            detail: "scalar missing",
        });
    };
    if !value.is_finite() || *value < 0.0 || value.fract() != 0.0 || *value > u64::MAX as f64 {
        return Err(WardError::InvalidFrequency {
            cx_id,
            value: *value,
        });
    }
    Ok(*value as u64)
}

fn expected_epoch(last_t: EpochSecs, cadence_secs: f64) -> Result<EpochSecs, WardError> {
    let expected = last_t.0 as f64 + cadence_secs;
    if !expected.is_finite() || expected < i64::MIN as f64 || expected > i64::MAX as f64 {
        return Err(WardError::InvalidDomain {
            reason: format!("expected recurrence time overflowed for cadence {cadence_secs}"),
        });
    }
    Ok(EpochSecs(expected.round() as i64))
}

fn unique_domain_ids(domain: &Domain) -> BTreeSet<CxId> {
    domain.cx_ids.iter().copied().collect()
}

fn ward_runtime(error: calyx_core::CalyxError) -> WardError {
    WardError::Runtime {
        reason: error.to_string(),
    }
}

fn derive_novel_id(
    profile: &GuardProfile,
    verdict: &GuardVerdict,
    produced: &ProducedSlots,
    ts: i64,
) -> NovelId {
    let mut hash = Sha256::new();
    hash.update(profile.guard_id.to_string().as_bytes());
    hash.update(profile.panel_version.to_be_bytes());
    hash.update(profile.domain.as_bytes());
    hash.update(ts.to_be_bytes());
    for (slot, values) in produced {
        hash.update(slot.get().to_be_bytes());
        for value in values {
            hash.update(value.to_bits().to_be_bytes());
        }
    }
    for slot in &verdict.per_slot {
        hash.update(slot.slot.get().to_be_bytes());
        hash.update(slot.cos.to_bits().to_be_bytes());
        hash.update(slot.tau.to_bits().to_be_bytes());
        hash.update([u8::from(slot.pass)]);
    }
    let digest = hash.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    NovelId::new(Uuid::from_bytes(bytes))
}
