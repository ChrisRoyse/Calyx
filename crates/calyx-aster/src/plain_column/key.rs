//! Key-encoding for the plain-collection wide-column layer.
//!
//! Two physical keys are maintained per cell so both access patterns are
//! `O(result)` (PRD `20 §2`, wide-column root op):
//!
//! * **Row-major (`KIND_CELL`)** — `disc | coll | CELL | row_len | row | column`
//!   The column is the *terminal* component (no length prefix), so columns of a
//!   row sort in pure lexicographic order — a Bigtable/HBase column-qualifier
//!   range scan within a row is a correct byte range.
//! * **Column-major (`KIND_COLINDEX`)** — `disc | coll | COLINDEX | col_len | col | row`
//!   The row is terminal, so a single column reads across rows in row order.
//!
//! Every key starts with the 1-byte discriminant `b'w'` (disjoint from the
//! plain-graph `b'g'` keyspace that shares the `Graph` CF) per PRD `04 §2`.
//! Length-prefixed components are `u16` big-endian, matching the existing
//! length-delimited convention in `plain_graph::key` and the 64KiB component
//! ceiling used by HBase/Bigtable.

use calyx_core::{CalyxError, Result};

use crate::cf::{KeyRange, prefix_range};

const DISC: u8 = b'w';
const KIND_CELL: u8 = 0;
const KIND_COLINDEX: u8 = 1;

pub(super) const MAX_COLLECTION_BYTES: usize = 256;
pub(super) const MAX_ROW_BYTES: usize = 4096;
pub(super) const MAX_COLUMN_BYTES: usize = 1024;
pub(super) const MAX_VALUE_BYTES: usize = 1 << 20;

/// Encodes the two physical keys of one wide-column collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ColumnKeyspace {
    collection: Vec<u8>,
}

impl ColumnKeyspace {
    pub(super) fn new(collection: &str) -> Result<Self> {
        Ok(Self {
            collection: validate_collection(collection)?,
        })
    }

    pub(super) fn collection_name(&self) -> String {
        String::from_utf8_lossy(&self.collection).to_string()
    }

    /// Row-major cell key: `disc | coll | CELL | row_len | row | column`.
    pub(super) fn cell_key(&self, row: &[u8], column: &[u8]) -> Result<Vec<u8>> {
        validate_row(row)?;
        validate_column(column)?;
        let mut key = self.kind_prefix(KIND_CELL);
        push_lp(&mut key, row);
        key.extend_from_slice(column);
        Ok(key)
    }

    /// Column-major index key: `disc | coll | COLINDEX | col_len | col | row`.
    pub(super) fn index_key(&self, column: &[u8], row: &[u8]) -> Result<Vec<u8>> {
        validate_row(row)?;
        validate_column(column)?;
        let mut key = self.kind_prefix(KIND_COLINDEX);
        push_lp(&mut key, column);
        key.extend_from_slice(row);
        Ok(key)
    }

    /// Prefix range over every column of one row (row-major).
    pub(super) fn row_range(&self, row: &[u8]) -> Result<KeyRange> {
        validate_row(row)?;
        let mut prefix = self.kind_prefix(KIND_CELL);
        push_lp(&mut prefix, row);
        Ok(prefix_range(&prefix))
    }

    /// Bounded `[start, end)` range over a column window within one row.
    pub(super) fn row_column_range(
        &self,
        row: &[u8],
        start_column: &[u8],
        end_column: &[u8],
    ) -> Result<KeyRange> {
        validate_row(row)?;
        validate_column(start_column)?;
        validate_column(end_column)?;
        if start_column >= end_column {
            return Err(invalid(format!(
                "wide-column range requires start_column < end_column ({} >= {})",
                String::from_utf8_lossy(start_column),
                String::from_utf8_lossy(end_column)
            )));
        }
        let mut prefix = self.kind_prefix(KIND_CELL);
        push_lp(&mut prefix, row);
        let mut start = prefix.clone();
        start.extend_from_slice(start_column);
        let mut end = prefix;
        end.extend_from_slice(end_column);
        Ok(KeyRange {
            start,
            end: Some(end),
        })
    }

    /// Prefix range over every row carrying one column (column-major).
    pub(super) fn column_range(&self, column: &[u8]) -> Result<KeyRange> {
        validate_column(column)?;
        let mut prefix = self.kind_prefix(KIND_COLINDEX);
        push_lp(&mut prefix, column);
        Ok(prefix_range(&prefix))
    }

    /// Decodes the `(row, column)` pair from a row-major cell key.
    pub(super) fn decode_cell_key(&self, key: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let prefix = self.kind_prefix(KIND_CELL);
        let body = key
            .strip_prefix(prefix.as_slice())
            .ok_or_else(|| corrupt("wide-column cell key has wrong prefix"))?;
        let (row, rest) = read_lp(body)?;
        if rest.is_empty() {
            return Err(corrupt("wide-column cell key is missing its column"));
        }
        Ok((row.to_vec(), rest.to_vec()))
    }

    /// Decodes the `(column, row)` pair from a column-major index key.
    pub(super) fn decode_index_key(&self, key: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let prefix = self.kind_prefix(KIND_COLINDEX);
        let body = key
            .strip_prefix(prefix.as_slice())
            .ok_or_else(|| corrupt("wide-column index key has wrong prefix"))?;
        let (column, rest) = read_lp(body)?;
        if rest.is_empty() {
            return Err(corrupt("wide-column index key is missing its row"));
        }
        Ok((column.to_vec(), rest.to_vec()))
    }

    fn collection_prefix(&self) -> Vec<u8> {
        let mut key = Vec::with_capacity(3 + self.collection.len());
        key.push(DISC);
        push_lp(&mut key, &self.collection);
        key
    }

    fn kind_prefix(&self, kind: u8) -> Vec<u8> {
        let mut key = self.collection_prefix();
        key.push(kind);
        key
    }
}

fn push_lp(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn read_lp(body: &[u8]) -> Result<(&[u8], &[u8])> {
    if body.len() < 2 {
        return Err(corrupt("wide-column key is shorter than its length prefix"));
    }
    let len = u16::from_be_bytes([body[0], body[1]]) as usize;
    let rest = &body[2..];
    if rest.len() < len {
        return Err(corrupt("wide-column key length prefix overruns the key"));
    }
    Ok((&rest[..len], &rest[len..]))
}

pub(super) fn validate_value(value: &[u8]) -> Result<()> {
    if value.len() > MAX_VALUE_BYTES {
        return Err(invalid(format!(
            "wide-column value exceeds {MAX_VALUE_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_collection(value: &str) -> Result<Vec<u8>> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_COLLECTION_BYTES || bytes.iter().any(|b| *b < 0x20) {
        return Err(invalid(
            "wide-column collection id must be printable and 1..=256 bytes",
        ));
    }
    Ok(bytes.to_vec())
}

fn validate_row(row: &[u8]) -> Result<()> {
    if row.is_empty() || row.len() > MAX_ROW_BYTES {
        return Err(invalid(format!(
            "wide-column row key must be 1..={MAX_ROW_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_column(column: &[u8]) -> Result<()> {
    if column.is_empty() || column.len() > MAX_COLUMN_BYTES {
        return Err(invalid(format!(
            "wide-column column name must be 1..={MAX_COLUMN_BYTES} bytes"
        )));
    }
    Ok(())
}

pub(super) fn invalid(message: impl Into<String>) -> CalyxError {
    wide_error("CALYX_WIDECOLUMN_INVALID_KEY", message)
}

pub(super) fn limit(message: impl Into<String>) -> CalyxError {
    wide_error("CALYX_WIDECOLUMN_SCAN_LIMIT", message)
}

pub(super) fn corrupt(message: impl Into<String>) -> CalyxError {
    wide_error("CALYX_WIDECOLUMN_CORRUPT_ROW", message)
}

fn wide_error(code: &'static str, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: "fix the wide-column key/value input or rebuild the plain collection",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_columns_sort_lexicographically_within_a_row() {
        let ks = ColumnKeyspace::new("plain").unwrap();
        let a = ks.cell_key(b"row1", b"age").unwrap();
        let b = ks.cell_key(b"row1", b"name").unwrap();
        let z = ks.cell_key(b"row1", b"zip").unwrap();
        assert_eq!(a[0], DISC);
        assert!(a < b, "age must sort before name");
        assert!(b < z, "name must sort before zip");
        // a longer row must not bleed into row1's column space.
        let other = ks.cell_key(b"row10", b"age").unwrap();
        assert!(z < other, "row1 cells must precede row10 cells");
    }

    #[test]
    fn index_rows_sort_lexicographically_for_a_column() {
        let ks = ColumnKeyspace::new("plain").unwrap();
        let r1 = ks.index_key(b"age", b"row1").unwrap();
        let r2 = ks.index_key(b"age", b"row2").unwrap();
        assert!(r1 < r2);
        // a different column is a disjoint prefix.
        let name = ks.index_key(b"name", b"row1").unwrap();
        assert!(r2 < name);
    }

    #[test]
    fn cell_key_round_trips_through_decode() {
        let ks = ColumnKeyspace::new("plain").unwrap();
        let key = ks.cell_key(b"row1", b"name").unwrap();
        let (row, column) = ks.decode_cell_key(&key).unwrap();
        assert_eq!(row, b"row1");
        assert_eq!(column, b"name");
    }

    #[test]
    fn index_key_round_trips_through_decode() {
        let ks = ColumnKeyspace::new("plain").unwrap();
        let key = ks.index_key(b"name", b"row1").unwrap();
        let (column, row) = ks.decode_index_key(&key).unwrap();
        assert_eq!(column, b"name");
        assert_eq!(row, b"row1");
    }

    #[test]
    fn distinct_collections_never_share_a_prefix() {
        let a = ColumnKeyspace::new("a").unwrap();
        let ab = ColumnKeyspace::new("ab").unwrap();
        let ka = a.cell_key(b"r", b"c").unwrap();
        let kab = ab.cell_key(b"r", b"c").unwrap();
        assert!(!ka.starts_with(&kab) && !kab.starts_with(&ka));
    }

    #[test]
    fn empty_and_oversized_inputs_fail_closed() {
        let ks = ColumnKeyspace::new("plain").unwrap();
        assert_eq!(
            ks.cell_key(b"", b"c").unwrap_err().code,
            "CALYX_WIDECOLUMN_INVALID_KEY"
        );
        assert_eq!(
            ks.cell_key(b"r", b"").unwrap_err().code,
            "CALYX_WIDECOLUMN_INVALID_KEY"
        );
        let big_row = vec![b'x'; MAX_ROW_BYTES + 1];
        assert_eq!(
            ks.cell_key(&big_row, b"c").unwrap_err().code,
            "CALYX_WIDECOLUMN_INVALID_KEY"
        );
        assert!(ColumnKeyspace::new("").is_err());
    }

    #[test]
    fn inverted_column_range_is_rejected() {
        let ks = ColumnKeyspace::new("plain").unwrap();
        assert_eq!(
            ks.row_column_range(b"r", b"m", b"a").unwrap_err().code,
            "CALYX_WIDECOLUMN_INVALID_KEY"
        );
    }
}
