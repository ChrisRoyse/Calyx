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
    /// Plain collection graph rows: nodes, typed edges, reverse index, CSR projection.
    Graph,
    /// Typed online/adaptation state.
    Online,
    /// Anneal rollback snapshots and live artifact pointers.
    AnnealRollback,
    /// Anneal component health snapshot.
    AnnealHealth,
    /// Anneal base-shard checksum and restore metadata.
    AnnealChecksums,
    /// Anneal online mistake-closure log.
    AnnealMistakes,
}

impl ColumnFamily {
    /// Static non-slot families in manifest order.
    pub const STATIC: [Self; 14] = [
        Self::Base,
        Self::XTerm,
        Self::TemporalXTerm,
        Self::Scalars,
        Self::Anchors,
        Self::Assay,
        Self::Ledger,
        Self::Recurrence,
        Self::Graph,
        Self::Online,
        Self::AnnealRollback,
        Self::AnnealHealth,
        Self::AnnealChecksums,
        Self::AnnealMistakes,
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
            Self::Graph => "graph".to_string(),
            Self::Online => "online".to_string(),
            Self::AnnealRollback => "anneal_rollback".to_string(),
            Self::AnnealHealth => "anneal_health".to_string(),
            Self::AnnealChecksums => "anneal_checksums".to_string(),
            Self::AnnealMistakes => "anneal_mistakes".to_string(),
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
