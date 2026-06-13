use calyx_core::Seq;
use serde::{Deserialize, Serialize};

/// One materialized wide-column cell read back from the store.
///
/// Absence is represented by a cell simply not appearing in a scan result —
/// there is never a zero-filled placeholder (PRD `20 §3`, sparse-by-design).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WideCell {
    pub row: Vec<u8>,
    pub column: Vec<u8>,
    pub value: Vec<u8>,
}

/// Receipt for a wide-column `put`, naming both physical keys it wrote so a
/// caller can read them back independently for FSV.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellCommit {
    pub seq: Seq,
    /// Row-major key under the `Graph` CF: `disc | coll | CELL | row_len | row | column`.
    pub cell_key: Vec<u8>,
    /// Column-major index key: `disc | coll | COLINDEX | col_len | col | row`.
    pub index_key: Vec<u8>,
}
