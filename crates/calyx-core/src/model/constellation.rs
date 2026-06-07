//! Atomic Calyx constellation record.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{CxId, Modality, SlotId, VaultId};

use super::{Anchor, CxFlags, InputRef, LedgerRef, SlotVector, Ts};

/// One input measured by one panel of frozen lenses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Constellation {
    /// Content-addressed constellation id.
    pub cx_id: CxId,
    /// Owning vault.
    pub vault_id: VaultId,
    /// Panel version used for this measurement.
    pub panel_version: u32,
    /// Server-stamped creation timestamp.
    pub created_at: Ts,
    /// Hash and optional raw-input pointer.
    pub input_ref: InputRef,
    /// Input modality.
    pub modality: Modality,
    /// Per-slot vectors; absent slots are explicit values.
    pub slots: BTreeMap<SlotId, SlotVector>,
    /// Scalar measurements derived at ingest.
    pub scalars: BTreeMap<String, f64>,
    /// Grounded outcomes observed for this input.
    pub anchors: Vec<Anchor>,
    /// Ledger entry proving input -> lens -> constellation lineage.
    pub provenance: LedgerRef,
    /// Trust and degradation flags for this constellation.
    pub flags: CxFlags,
}
