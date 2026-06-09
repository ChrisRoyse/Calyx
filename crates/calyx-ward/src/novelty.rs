//! Novelty routing for failed Ward verdicts.

use std::fmt;
use std::sync::Arc;

use calyx_core::Clock;
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
