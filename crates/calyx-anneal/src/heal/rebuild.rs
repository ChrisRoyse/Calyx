mod artifact;
mod builders;
mod scheduler;
mod source;

use calyx_core::{Result, Seq, SlotId};
use serde::{Deserialize, Serialize};

use crate::{ArtifactKey, ArtifactPtr, BudgetHandle, ChangeId, ComponentKind, ScopeId};

pub use builders::{AnnIndexRebuilder, GuardProfileRebuilder, KernelIndexRebuilder};
pub use scheduler::RebuildScheduler;
pub use source::AsterRebuildSource;

pub type MvccSnapshot = Seq;

pub const CALYX_ANNEAL_REBUILD_SOURCE_VIOLATION: &str = "CALYX_ANNEAL_REBUILD_SOURCE_VIOLATION";
pub const CALYX_ASTER_SNAPSHOT_UNAVAILABLE: &str = "CALYX_ASTER_SNAPSHOT_UNAVAILABLE";
pub const CALYX_ANNEAL_REBUILD_IO: &str = "CALYX_ANNEAL_REBUILD_IO";
pub const CALYX_ANNEAL_REBUILD_INVALID_TARGET: &str = "CALYX_ANNEAL_REBUILD_INVALID_TARGET";
pub const CALYX_ANNEAL_REBUILD_TRIPWIRE_FAILED: &str = "CALYX_ANNEAL_REBUILD_TRIPWIRE_FAILED";

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RebuildTarget {
    AnnIndex { slot_id: SlotId },
    KernelIndex { scope: ScopeId },
    GuardProfile { slot_id: SlotId },
}

impl RebuildTarget {
    pub fn component(&self) -> ComponentKind {
        match self {
            Self::AnnIndex { slot_id } => ComponentKind::AnnIndex { slot_id: *slot_id },
            Self::KernelIndex { scope } => ComponentKind::KernelIndex {
                scope: scope.clone(),
            },
            Self::GuardProfile { slot_id } => ComponentKind::GuardProfile { slot_id: *slot_id },
        }
    }

    pub fn artifact_key(&self) -> ArtifactKey {
        let hash = artifact::target_hash(self);
        match self {
            Self::AnnIndex { .. } => ArtifactKey::HnswGraph(hash),
            Self::KernelIndex { .. } => ArtifactKey::QuantLevel(hash),
            Self::GuardProfile { .. } => ArtifactKey::ConfigCache(hash),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RebuildPriority(pub u8);

impl RebuildPriority {
    pub const LOW: Self = Self(32);
    pub const NORMAL: Self = Self(128);
    pub const HIGH: Self = Self(224);
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebuildJob {
    pub target: RebuildTarget,
    pub priority: RebuildPriority,
    pub(crate) sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RebuildOutcome {
    Completed {
        change_id: ChangeId,
        prior_ptr: ArtifactPtr,
        new_ptr: ArtifactPtr,
    },
    Failed {
        target: RebuildTarget,
        reason_code: String,
        reason: String,
    },
    BudgetExhausted {
        target: RebuildTarget,
    },
    SkippedNotDegraded {
        target: RebuildTarget,
    },
    NothingQueued,
}

pub trait Rebuilder: Send + Sync {
    fn rebuild(
        &self,
        target: &RebuildTarget,
        snapshot: MvccSnapshot,
        budget: &mut BudgetHandle,
    ) -> Result<ArtifactPtr>;
}

pub(crate) fn invalid_target(message: impl Into<String>) -> calyx_core::CalyxError {
    calyx_core::CalyxError {
        code: CALYX_ANNEAL_REBUILD_INVALID_TARGET,
        message: message.into(),
        remediation: "enqueue only derived rebuild targets with installed live pointers",
    }
}
