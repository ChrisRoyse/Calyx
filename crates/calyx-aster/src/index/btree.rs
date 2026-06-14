//! Btree secondary-index key encoding (PH54 T01, discriminant `0x10`).
//!
//! Key schema (value is empty — existence is the signal):
//! ```text
//! 0x10 | collection_id (8B BE) | index_id (4B BE) | field_val_encoded | pk_bytes
//! ```
//! `field_val_encoded` is *memcomparable*: lexicographic byte order equals the
//! natural order of the indexed value, so a forward range scan over the keyspace
//! yields rows in field order. The primary key trails as a tiebreaker for
//! non-unique indexes; it never participates in field ordering because every
//! `field_val_encoded` form is self-delimiting (fixed width, or escape-
//! terminated for variable-length text/bytes).
//!
//! Per-type encoding:
//! * `I64`: sign-flip (`XOR 0x8000_0000_0000_0000`) then 8B BE.
//! * `U64`: plain 8B BE, preserving the full unsigned range.
//! * `Timestamp`: non-negative i64 nanoseconds as 8B BE.
//! * `F64`: IEEE-754 total-order transform then 8B BE.
//! * `Bool`: single byte `0x00`/`0x01`.
//! * `Text`/`Bytes`: first 64 bytes, escape-terminated memcomparable form.
//!
//! NULL is not indexable in a btree key (no type, no defined order) and fails
//! closed; per-field NULL handling via a leading flag byte is a later phase.

use calyx_core::{CalyxError, Clock, Result, Seq};

use super::maintenance::CALYX_INDEX_STALE_ENTRY;
use super::{
    FieldValue, IndexKind, IndexSpec, SecondaryIndex, field_value_type, invalid_index_input,
};
use crate::cf::{ColumnFamily, KeyRange, prefix_range};
use crate::collection::{CALYX_INVALID_ARGUMENT, Collection, CollectionMode, FieldType};
use crate::layers::document::{DocId, document_prefix};
use crate::layers::relational::{collection_id, record_key};
use crate::layers::{RecordKey, RecordValue};
use crate::vault::AsterVault;

/// Key-space discriminant for btree secondary-index keys.
pub const DISC_BTREE_INDEX: u8 = 0x10;

/// On-disk name of the Aster CF that stores btree secondary-index entries.
/// Must equal [`ColumnFamily::IndexBtree`]'s `name()` (asserted in tests).
pub const CF_INDEX_BTREE: &str = "index_btree";

/// `0x10` + collection_id (8B) + index_id (4B).
const PREFIX_BYTES: usize = 1 + 8 + 4;
/// Maximum bytes of a Text/Bytes value retained in the index key (prefix index).
pub const MAX_INDEXED_BYTES: usize = 64;
/// Sign bit of a 64-bit word.
const SIGN_BIT: u64 = 1 << 63;
/// Memcomparable escape byte and its two trailers.
const ESC: u8 = 0x00;
const ESC_LITERAL: u8 = 0xff; // `0x00 0xff` decodes to a literal 0x00.
const ESC_TERM: u8 = 0x01; // `0x00 0x01` terminates a variable-length value.

/// A btree secondary index over one field of one collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BtreeIndex {
    collection_id: u64,
    spec: IndexSpec,
}

impl BtreeIndex {
    /// Builds a btree index bound to a collection id and runtime spec.
    pub fn new(collection_id: u64, spec: IndexSpec) -> Self {
        Self {
            collection_id,
            spec,
        }
    }

    pub fn spec(&self) -> &IndexSpec {
        &self.spec
    }

    /// The 13-byte fixed key prefix (discriminant + collection id + index id)
    /// that bounds this index's entire keyspace — the lower bound of an
    /// unbounded range scan.
    pub fn index_key_prefix(&self) -> Vec<u8> {
        self.prefix()
    }

    /// The 13-byte fixed key prefix: discriminant + collection id + index id.
    fn prefix(&self) -> Vec<u8> {
        let mut prefix = Vec::with_capacity(PREFIX_BYTES);
        prefix.push(DISC_BTREE_INDEX);
        prefix.extend_from_slice(&self.collection_id.to_be_bytes());
        prefix.extend_from_slice(&self.spec.index_id.to_be_bytes());
        prefix
    }

    /// Validates `field_val` against the index's declared type, then returns its
    /// memcomparable encoding.
    fn encode_field_value(&self, field_val: &FieldValue) -> Result<Vec<u8>> {
        let actual = field_value_type(field_val)
            .ok_or_else(|| invalid_index_input("cannot index a NULL value in a btree key"))?;
        if actual != self.spec.field_type {
            return Err(invalid_index_input(format!(
                "field value type {actual:?} does not match index field type {:?}",
                self.spec.field_type
            )));
        }
        match field_val {
            RecordValue::Bool(value) => Ok(vec![u8::from(*value)]),
            RecordValue::I64(value) => Ok(i64_order_bytes(*value).to_vec()),
            RecordValue::U64(value) => Ok(value.to_be_bytes().to_vec()),
            RecordValue::Timestamp(value) => {
                if *value < 0 {
                    return Err(invalid_index_input(
                        "timestamp index value must be non-negative nanoseconds",
                    ));
                }
                Ok((*value as u64).to_be_bytes().to_vec())
            }
            RecordValue::F64(value) => {
                if !value.is_finite() {
                    return Err(invalid_index_input("F64 index value must be finite"));
                }
                Ok(f64_order_bits(*value).to_be_bytes().to_vec())
            }
            RecordValue::Text(value) => Ok(encode_memcomparable(truncate_utf8(value).as_bytes())),
            RecordValue::Bytes(value) => Ok(encode_memcomparable(
                &value[..value.len().min(MAX_INDEXED_BYTES)],
            )),
            // Unreachable: `field_value_type` already mapped this to a type that
            // matched `field_type`; NULL was rejected above.
            RecordValue::Null => Err(invalid_index_input("cannot index a NULL value")),
        }
    }

    /// Reverses [`Self::encode_field_value`] for the index's declared type,
    /// returning the value and the number of body bytes it consumed. Trailing
    /// bytes are the primary key.
    fn decode_field_value(&self, body: &[u8]) -> Result<(FieldValue, usize)> {
        match self.spec.field_type {
            FieldType::Bool => {
                let byte = *body
                    .first()
                    .ok_or_else(|| corrupt("btree bool key truncated"))?;
                let value = match byte {
                    0 => false,
                    1 => true,
                    other => return Err(corrupt(format!("btree bool byte {other} not 0/1"))),
                };
                Ok((RecordValue::Bool(value), 1))
            }
            FieldType::I64 => {
                let word = read_u64(body, "i64")?;
                Ok((RecordValue::I64((word ^ SIGN_BIT) as i64), 8))
            }
            FieldType::U64 => Ok((RecordValue::U64(read_u64(body, "u64")?), 8)),
            FieldType::Timestamp => {
                let word = read_u64(body, "timestamp")?;
                if word > i64::MAX as u64 {
                    return Err(corrupt("btree timestamp exceeds i64::MAX"));
                }
                Ok((RecordValue::Timestamp(word as i64), 8))
            }
            FieldType::F64 => {
                let word = read_u64(body, "f64")?;
                Ok((RecordValue::F64(f64_from_order_bits(word)), 8))
            }
            FieldType::Text => {
                let (bytes, used) = decode_memcomparable(body)?;
                let text = String::from_utf8(bytes)
                    .map_err(|error| corrupt(format!("btree text not valid UTF-8: {error}")))?;
                Ok((RecordValue::Text(text), used))
            }
            FieldType::Bytes => {
                let (bytes, used) = decode_memcomparable(body)?;
                Ok((RecordValue::Bytes(bytes), used))
            }
        }
    }

    /// Decodes a full index key back into `(field value, primary key)`. Fails
    /// closed with `CALYX_ASTER_CORRUPT_SHARD` on any malformed key.
    pub fn decode_index_key(&self, key: &[u8]) -> Result<(FieldValue, RecordKey)> {
        let prefix = self.prefix();
        if key.len() < prefix.len() || key[..prefix.len()] != prefix[..] {
            return Err(corrupt(
                "btree index key prefix does not match this index keyspace",
            ));
        }
        let body = &key[prefix.len()..];
        let (field_val, consumed) = self.decode_field_value(body)?;
        let pk_bytes = &body[consumed..];
        let pk = RecordKey::from_bytes(pk_bytes.to_vec())
            .map_err(|error| corrupt(format!("btree index key primary key: {error}")))?;
        Ok((field_val, pk))
    }
}

impl SecondaryIndex for BtreeIndex {
    fn kind(&self) -> IndexKind {
        self.spec.kind
    }

    fn encode_index_key(&self, field_val: &FieldValue, pk: &RecordKey) -> Result<Vec<u8>> {
        let encoded = self.encode_field_value(field_val)?;
        let mut key = self.prefix();
        key.extend_from_slice(&encoded);
        key.extend_from_slice(pk.as_bytes());
        Ok(key)
    }

    fn encode_scan_prefix(&self, field_val: &FieldValue) -> Result<Vec<u8>> {
        let encoded = self.encode_field_value(field_val)?;
        let mut prefix = self.prefix();
        prefix.extend_from_slice(&encoded);
        Ok(prefix)
    }
}

/// Sign-flipped big-endian form of an i64 (negatives sort first).
fn i64_order_bytes(value: i64) -> [u8; 8] {
    ((value as u64) ^ SIGN_BIT).to_be_bytes()
}

/// IEEE-754 total-order bits: negatives flip all bits, non-negatives flip sign.
///
/// `-0.0` is canonicalized to `+0.0` first so the two numerically-equal zeros
/// encode to the *same* key — otherwise a lookup for `0.0` would miss a row
/// indexed under `-0.0`. NaN is rejected by the caller before reaching here.
fn f64_order_bits(value: f64) -> u64 {
    let value = if value == 0.0 { 0.0 } else { value };
    let bits = value.to_bits();
    if bits & SIGN_BIT != 0 {
        !bits
    } else {
        bits | SIGN_BIT
    }
}

/// Inverse of [`f64_order_bits`].
fn f64_from_order_bits(encoded: u64) -> f64 {
    let bits = if encoded & SIGN_BIT != 0 {
        encoded & !SIGN_BIT
    } else {
        !encoded
    };
    f64::from_bits(bits)
}

/// Truncates `s` to at most [`MAX_INDEXED_BYTES`] bytes on a char boundary.
fn truncate_utf8(s: &str) -> &str {
    if s.len() <= MAX_INDEXED_BYTES {
        return s;
    }
    let mut end = MAX_INDEXED_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Order-preserving, self-delimiting encoding of an arbitrary byte string:
/// each `0x00` becomes `0x00 0xff`; the value ends with the `0x00 0x01`
/// terminator. A value that is a byte-prefix of another always sorts first
/// because the terminator's `0x01` is below any escaped content's `0xff` and
/// below any non-zero content byte.
fn encode_memcomparable(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 2);
    for &byte in data {
        out.push(byte);
        if byte == ESC {
            out.push(ESC_LITERAL);
        }
    }
    out.push(ESC);
    out.push(ESC_TERM);
    out
}

/// Inverse of [`encode_memcomparable`]: returns the decoded bytes and the number
/// of input bytes consumed (through the terminator).
fn decode_memcomparable(input: &[u8]) -> Result<(Vec<u8>, usize)> {
    let mut data = Vec::new();
    let mut i = 0;
    while i < input.len() {
        let byte = input[i];
        if byte == ESC {
            let next = *input
                .get(i + 1)
                .ok_or_else(|| corrupt("btree memcomparable value: trailing escape byte"))?;
            match next {
                ESC_LITERAL => {
                    data.push(ESC);
                    i += 2;
                }
                ESC_TERM => return Ok((data, i + 2)),
                other => {
                    return Err(corrupt(format!(
                        "btree memcomparable value: invalid escape 0x00 {other:#04x}"
                    )));
                }
            }
        } else {
            data.push(byte);
            i += 1;
        }
    }
    Err(corrupt("btree memcomparable value: missing terminator"))
}

fn read_u64(body: &[u8], label: &str) -> Result<u64> {
    let slice = body
        .get(..8)
        .ok_or_else(|| corrupt(format!("btree {label} key truncated (need 8 bytes)")))?;
    Ok(u64::from_be_bytes(slice.try_into().expect("8-byte slice")))
}

fn corrupt(message: impl Into<String>) -> CalyxError {
    CalyxError::aster_corrupt_shard(message)
}

// ── Range / point / count queries over the `index_btree` CF ─────────────────
//
// All queries pin a single MVCC snapshot (`vault.latest_seq()`), so a query
// sees one consistent committed state with no dirty reads. An index key whose
// data row is absent at that snapshot is a *stale* entry (the row was deleted
// but the index not yet compacted — handled by PH54 T05); it is skipped, never
// returned. The pre-T04 write path does not yet maintain the index, so callers
// (and tests) populate entries via [`btree_index_put`].

fn index_for(col: &Collection, spec: &IndexSpec) -> BtreeIndex {
    BtreeIndex::new(collection_id(col), spec.clone())
}

/// Builds the half-open `[start, end)` index-key range for an inclusive
/// `[gte, lte]` field-value range. `None` bounds extend to this index's full
/// keyspace. `lte` is made inclusive by taking the lexicographic upper bound of
/// its exact-value scan prefix.
fn range_bounds(
    idx: &BtreeIndex,
    gte: Option<&FieldValue>,
    lte: Option<&FieldValue>,
) -> Result<KeyRange> {
    let start = match gte {
        Some(value) => idx.encode_scan_prefix(value)?,
        None => idx.index_key_prefix(),
    };
    let end = match lte {
        Some(value) => prefix_range(&idx.encode_scan_prefix(value)?).end,
        None => prefix_range(&idx.index_key_prefix()).end,
    };
    Ok(KeyRange { start, end })
}

/// Writes one index entry `(field value, pk) -> ∅` for `spec` over `col`.
/// Direct index write used by tests and (later) the T04 maintenance hook.
pub fn btree_index_put<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    field_val: &FieldValue,
    pk: &RecordKey,
) -> Result<Seq> {
    let key = index_for(col, spec).encode_index_key(field_val, pk)?;
    vault.write_cf(ColumnFamily::IndexBtree, key, Vec::new())
}

/// Range query at an explicit snapshot. Returns the primary keys whose indexed
/// field value lies in `[gte, lte]`, in ascending index order, skipping stale
/// entries. `limit == 0` means unbounded (caller must bound for huge indexes).
pub fn btree_range_at<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    col: &Collection,
    spec: &IndexSpec,
    gte: Option<&FieldValue>,
    lte: Option<&FieldValue>,
    limit: usize,
) -> Result<Vec<RecordKey>> {
    let idx = index_for(col, spec);
    let range = range_bounds(&idx, gte, lte)?;
    let mut out = Vec::new();
    for (key, _empty) in vault.scan_cf_range_at(snapshot, ColumnFamily::IndexBtree, &range)? {
        if limit != 0 && out.len() == limit {
            break;
        }
        let (_field, pk) = idx.decode_index_key(&key)?;
        if pk_is_live(vault, snapshot, col, &pk)? {
            out.push(pk);
        } else {
            warn_stale_entry(col, spec, &pk);
        }
    }
    Ok(out)
}

/// Range query at the latest committed snapshot. See [`btree_range_at`].
pub fn btree_range<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    gte: Option<&FieldValue>,
    lte: Option<&FieldValue>,
    limit: usize,
) -> Result<Vec<RecordKey>> {
    btree_range_at(vault, vault.latest_seq(), col, spec, gte, lte, limit)
}

/// Point query: every primary key whose indexed field value equals `val`.
pub fn btree_point<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    val: &FieldValue,
) -> Result<Vec<RecordKey>> {
    btree_range(vault, col, spec, Some(val), Some(val), 0)
}

/// Counts live (non-stale) entries in `[gte, lte]` without materializing the
/// primary keys. Stale entries are excluded so the count agrees with
/// [`btree_range`]'s length.
pub fn btree_count<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    gte: Option<&FieldValue>,
    lte: Option<&FieldValue>,
) -> Result<u64> {
    let snapshot = vault.latest_seq();
    let idx = index_for(col, spec);
    let range = range_bounds(&idx, gte, lte)?;
    let mut count = 0_u64;
    for (key, _empty) in vault.scan_cf_range_at(snapshot, ColumnFamily::IndexBtree, &range)? {
        let (_field, pk) = idx.decode_index_key(&key)?;
        if pk_is_live(vault, snapshot, col, &pk)? {
            count += 1;
        } else {
            warn_stale_entry(col, spec, &pk);
        }
    }
    Ok(count)
}

/// Resolves whether the data row behind an index entry's primary key is still
/// live at `snapshot`, reading the column family that the owning collection's
/// paradigm layer actually stores data in.
///
/// PH54 T04 maintains secondary indexes for every paradigm layer, so the
/// liveness check that filters stale entries must dispatch on collection mode —
/// assuming the relational CF would treat every KV/TS/Document index entry as
/// stale and silently drop it. `read_cf_at` already filters MVCC tombstones, so
/// a deleted row reads back as absent.
fn pk_is_live<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    col: &Collection,
    pk: &RecordKey,
) -> Result<bool> {
    match col.mode {
        CollectionMode::Records => {
            let data_key = record_key(col, pk)?;
            Ok(vault
                .read_cf_at(snapshot, ColumnFamily::Relational, &data_key)?
                .is_some())
        }
        // KV and TimeSeries index entries carry the full data key as their pk.
        CollectionMode::KV => Ok(vault
            .read_cf_at(snapshot, ColumnFamily::Kv, pk.as_bytes())?
            .is_some()),
        CollectionMode::TimeSeries => Ok(vault
            .read_cf_at(snapshot, ColumnFamily::TimeSeries, pk.as_bytes())?
            .is_some()),
        // A document is live if any leaf survives under its id prefix.
        CollectionMode::Documents => {
            let prefix = document_prefix(col, DocId::from_slice(pk.as_bytes())?);
            Ok(!vault
                .scan_cf_range_at(snapshot, ColumnFamily::Document, &prefix_range(&prefix))?
                .is_empty())
        }
        // Blob and Constellations route through the constellation/lens path and
        // never write btree index entries, so a query reaching here is a bug.
        other => Err(CalyxError {
            code: CALYX_INVALID_ARGUMENT,
            message: format!("btree index queries are not defined for {other:?} collections"),
            remediation: "query a Records/KV/TimeSeries/Documents collection",
        }),
    }
}

fn warn_stale_entry(col: &Collection, spec: &IndexSpec, pk: &RecordKey) {
    tracing::warn!(
        code = CALYX_INDEX_STALE_ENTRY,
        collection = %col.name,
        index = %spec.name,
        pk_hash = %blake3::hash(pk.as_bytes()).to_hex(),
        "stale btree index entry skipped"
    );
}

#[cfg(test)]
#[path = "btree_tests.rs"]
mod tests;
