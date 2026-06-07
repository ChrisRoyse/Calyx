//! Slot vector representations.

use serde::{Deserialize, Serialize};

use crate::AbsentReason;

/// Sparse vector entry.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SparseEntry {
    /// Ambient vector index.
    pub idx: u32,
    /// Entry value.
    pub val: f32,
}

/// Per-slot vector payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotVector {
    /// Dense f32 payload.
    Dense { dim: u32, data: Vec<f32> },
    /// Sparse payload.
    Sparse { dim: u32, entries: Vec<SparseEntry> },
    /// Multi-vector token payload.
    Multi {
        token_dim: u32,
        tokens: Vec<Vec<f32>>,
    },
    /// Explicit absence; this must never be interpreted as a zero vector.
    Absent { reason: AbsentReason },
}

impl SlotVector {
    /// Returns true when the vector is explicitly absent.
    pub const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent { .. })
    }

    /// Returns dense data only for a real dense vector.
    pub fn as_dense(&self) -> Option<&[f32]> {
        match self {
            Self::Dense { data, .. } => Some(data.as_slice()),
            Self::Sparse { .. } | Self::Multi { .. } | Self::Absent { .. } => None,
        }
    }
}
