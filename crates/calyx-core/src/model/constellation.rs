//! Atomic Calyx constellation record.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{CxId, Modality, SlotId, VaultId};

use super::{Anchor, CxFlags, InputRef, LedgerRef, SlotVector, Ts};

/// Leapable Vault contract key for a source chunk identifier.
pub const METADATA_CHUNK_ID: &str = "chunk_id";
/// Leapable Vault contract key for the owning database identifier.
pub const METADATA_DATABASE_NAME: &str = "database_name";

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
    /// Verbatim string identifiers and source-system metadata.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// Grounded outcomes observed for this input.
    pub anchors: Vec<Anchor>,
    /// Ledger entry proving input -> lens -> constellation lineage.
    pub provenance: LedgerRef,
    /// Trust and degradation flags for this constellation.
    pub flags: CxFlags,
}

impl Constellation {
    /// Returns a string metadata value without allocating.
    pub fn metadata_value(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).map(String::as_str)
    }

    /// Returns the preserved Leapable chunk identifier, when this row came from a Vault chunk.
    pub fn chunk_id(&self) -> Option<&str> {
        self.metadata_value(METADATA_CHUNK_ID)
    }

    /// Returns the preserved Leapable database identifier, when this row came from a Vault chunk.
    pub fn database_name(&self) -> Option<&str> {
        self.metadata_value(METADATA_DATABASE_NAME)
    }
}
