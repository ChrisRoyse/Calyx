//! Sparse wide-column root op for plain (0-lens) collections.
//!
//! A plain collection has no slot/scalar map (that structure only exists in
//! Constellations mode), so PRD `20 §2`'s wide-column claim — every plain
//! collection behaves as a real wide-column store — is delivered here as a
//! key-encoding layer over the ordered Aster core, alongside the plain-graph
//! layer. Rows live in the shared `Graph` CF under a disjoint `b'w'`
//! discriminant (PRD `04 §2`); no new column family or on-disk format change.
//!
//! Each cell is stored under two keys written in one group-commit batch:
//! a **row-major** key (`get` a cell, scan a row, scan a column window of a row)
//! and a **column-major** index (scan one column across every row). Both keep
//! the cell value so every access pattern is a single bounded scan. Sparsity is
//! physical: a row that lacks a column has no key, so a column scan returns only
//! the rows that actually carry it — absent cells are never zero-filled.

mod key;
mod types;

use calyx_core::{Clock, Result, Seq};

use crate::cf::ColumnFamily;
use crate::vault::AsterVault;
use key::{ColumnKeyspace, corrupt, limit, validate_value};

pub use types::{CellCommit, WideCell};

/// Wide-column accessor bound to one plain collection over an `AsterVault`.
pub struct PlainColumn<'a, C: Clock> {
    vault: &'a AsterVault<C>,
    keys: ColumnKeyspace,
}

impl<'a, C: Clock> PlainColumn<'a, C> {
    /// Opens the wide-column layer for `collection`.
    pub fn new(vault: &'a AsterVault<C>, collection: &str) -> Result<Self> {
        Ok(Self {
            vault,
            keys: ColumnKeyspace::new(collection)?,
        })
    }

    /// Name of the bound collection.
    pub fn collection(&self) -> String {
        self.keys.collection_name()
    }

    /// Writes (or overwrites) one cell, atomically updating both the row-major
    /// and column-major keys so the two indexes can never disagree.
    pub fn put(&self, row: &[u8], column: &[u8], value: &[u8]) -> Result<CellCommit> {
        validate_value(value)?;
        let cell_key = self.keys.cell_key(row, column)?;
        let index_key = self.keys.index_key(column, row)?;
        let seq = self.vault.write_cf_batch([
            (ColumnFamily::Graph, cell_key.clone(), value.to_vec()),
            (ColumnFamily::Graph, index_key.clone(), value.to_vec()),
        ])?;
        Ok(CellCommit {
            seq,
            cell_key,
            index_key,
        })
    }

    /// Reads a single cell. Returns `None` when the cell is absent — an explicit
    /// absence, never a zero-filled value.
    pub fn get(&self, snapshot: Seq, row: &[u8], column: &[u8]) -> Result<Option<Vec<u8>>> {
        let cell_key = self.keys.cell_key(row, column)?;
        self.vault
            .read_cf_at(snapshot, ColumnFamily::Graph, &cell_key)
    }

    /// Reads every column present on `row`, in lexicographic column order. Only
    /// columns that physically exist are returned (sparse, never zero-filled).
    pub fn scan_row(&self, snapshot: Seq, row: &[u8], limit_rows: usize) -> Result<Vec<WideCell>> {
        let range = self.keys.row_range(row)?;
        let rows = self
            .vault
            .scan_cf_range_at(snapshot, ColumnFamily::Graph, &range)?;
        enforce_limit(rows.len(), limit_rows, "wide-column row scan")?;
        rows.into_iter()
            .map(|(k, value)| {
                let (row, column) = self.keys.decode_cell_key(&k)?;
                Ok(WideCell { row, column, value })
            })
            .collect()
    }

    /// Reads the columns of `row` in the half-open window `[start_column,
    /// end_column)`, in lexicographic column order.
    pub fn scan_row_columns(
        &self,
        snapshot: Seq,
        row: &[u8],
        start_column: &[u8],
        end_column: &[u8],
        limit_rows: usize,
    ) -> Result<Vec<WideCell>> {
        let range = self.keys.row_column_range(row, start_column, end_column)?;
        let rows = self
            .vault
            .scan_cf_range_at(snapshot, ColumnFamily::Graph, &range)?;
        enforce_limit(rows.len(), limit_rows, "wide-column row-window scan")?;
        rows.into_iter()
            .map(|(k, value)| {
                let (row, column) = self.keys.decode_cell_key(&k)?;
                Ok(WideCell { row, column, value })
            })
            .collect()
    }

    /// Reads `column` across every row that carries it, in lexicographic row
    /// order. Rows without the column are simply absent from the result — the
    /// sparse-column-range root op of PRD `20 §2`.
    pub fn scan_column(
        &self,
        snapshot: Seq,
        column: &[u8],
        limit_rows: usize,
    ) -> Result<Vec<WideCell>> {
        let range = self.keys.column_range(column)?;
        let rows = self
            .vault
            .scan_cf_range_at(snapshot, ColumnFamily::Graph, &range)?;
        enforce_limit(rows.len(), limit_rows, "wide-column column scan")?;
        rows.into_iter()
            .map(|(k, value)| {
                let (decoded_column, row) = self.keys.decode_index_key(&k)?;
                if decoded_column != column {
                    return Err(corrupt(
                        "wide-column index row decoded a column outside the scan prefix",
                    ));
                }
                Ok(WideCell {
                    row,
                    column: decoded_column,
                    value,
                })
            })
            .collect()
    }
}

fn enforce_limit(found: usize, limit_rows: usize, what: &str) -> Result<()> {
    if found > limit_rows {
        return Err(limit(format!(
            "{what} matched {found} rows, exceeding the bound of {limit_rows}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
