//! Column-family identity and on-disk names.

use calyx_core::SlotId;

/// Per-slot column family flavor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SlotFamilyKind {
    /// Quantized, active slot vector column.
    Quantized,
    /// Raw f32 sidecar used for cold-tier rescore/re-quantization.
    Raw,
}

/// Aster column families from PRD 04 section 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColumnFamily {
    /// `CxId -> ConstellationHeader`.
    Base,
    /// Per-slot vector column, either quantized or raw sidecar.
    Slot { slot: SlotId, kind: SlotFamilyKind },
    /// `(CxId, a, b, kind) -> cross-term value`.
    XTerm,
    /// `(CxId_a, CxId_b) -> temporal cross-term value`.
    TemporalXTerm,
    /// `(ScalarId, CxId) -> f64`.
    Scalars,
    /// `(CxId, AnchorKind) -> AnchorValue + source + ts`.
    Anchors,
    /// `(panel_version, corpus_shard, subject) -> AssayRow`.
    Assay,
    /// `seq -> hash-chained provenance entry`.
    Ledger,
    /// `(CxId, OccurrenceId) -> recurrence occurrence or summary`.
    Recurrence,
    /// Typed online/adaptation state.
    Online,
    /// Anneal rollback snapshots and live artifact pointers.
    AnnealRollback,
}

impl ColumnFamily {
    /// Static non-slot families in manifest order.
    pub const STATIC: [Self; 10] = [
        Self::Base,
        Self::XTerm,
        Self::TemporalXTerm,
        Self::Scalars,
        Self::Anchors,
        Self::Assay,
        Self::Ledger,
        Self::Recurrence,
        Self::Online,
        Self::AnnealRollback,
    ];

    /// Creates a quantized slot column family such as `slot_00`.
    pub const fn slot(slot: SlotId) -> Self {
        Self::Slot {
            slot,
            kind: SlotFamilyKind::Quantized,
        }
    }

    /// Creates a raw sidecar slot column family such as `slot_00.raw`.
    pub const fn slot_raw(slot: SlotId) -> Self {
        Self::Slot {
            slot,
            kind: SlotFamilyKind::Raw,
        }
    }

    /// Returns the stable directory name under `vault/cf/`.
    pub fn name(&self) -> String {
        match self {
            Self::Base => "base".to_string(),
            Self::Slot {
                slot,
                kind: SlotFamilyKind::Quantized,
            } => format!("slot_{:02}", slot.get()),
            Self::Slot {
                slot,
                kind: SlotFamilyKind::Raw,
            } => format!("slot_{:02}.raw", slot.get()),
            Self::XTerm => "xterm".to_string(),
            Self::TemporalXTerm => "temporal_xterm".to_string(),
            Self::Scalars => "scalars".to_string(),
            Self::Anchors => "anchors".to_string(),
            Self::Assay => "assay".to_string(),
            Self::Ledger => "ledger".to_string(),
            Self::Recurrence => "recurrence".to_string(),
            Self::Online => "online".to_string(),
            Self::AnnealRollback => "anneal_rollback".to_string(),
        }
    }

    /// Returns true for slot CFs, including raw sidecars.
    pub const fn is_slot(&self) -> bool {
        matches!(self, Self::Slot { .. })
    }

    /// Returns true for raw f32 sidecar slot CFs.
    pub const fn is_raw_slot(&self) -> bool {
        matches!(
            self,
            Self::Slot {
                kind: SlotFamilyKind::Raw,
                ..
            }
        )
    }

    /// Returns the slot id for slot CFs.
    pub const fn slot_id(&self) -> Option<SlotId> {
        match self {
            Self::Slot { slot, .. } => Some(*slot),
            _ => None,
        }
    }
}
