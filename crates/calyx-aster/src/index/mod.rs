//! Secondary-index trait, runtime `IndexSpec`, and index-value types (PH54 T01).
//!
//! PH53 declared *which* indexes a collection carries via
//! [`SecondaryIndexSpec`](crate::collection::SecondaryIndexSpec) (name + kind +
//! field names). PH54 turns that declaration into a *runtime* index: a stable
//! [`IndexId`], the resolved [`FieldType`] of the indexed field, and an
//! implementation that maps `(field value, primary key)` onto the Aster ordered
//! keyspace so range and point scans are correct byte-for-byte.
//!
//! The first implementation is [`btree::BtreeIndex`] (discriminant `0x10`). Its
//! key encoding is *memcomparable*: lexicographic byte order over the encoded
//! key equals the natural order of the indexed values. This is the same
//! discipline TiKV/MyRocks use for secondary indexes — index-number prefix,
//! then the order-preserving form of the field value, then the primary key as a
//! tiebreaker, with an empty value (existence is the signal).
//!
//! Field values reuse [`RecordValue`](crate::layers::RecordValue) (aliased here
//! as [`FieldValue`]) and primary keys reuse
//! [`RecordKey`](crate::layers::RecordKey) — the relational layer's existing
//! types — rather than introducing parallel definitions.

pub mod btree;
pub mod inverted;
pub mod maintenance;
pub mod rebuild;
mod terms;

pub use btree::{BtreeIndex, DISC_BTREE_INDEX};
pub use inverted::{DISC_INVERTED_INDEX, InvertedIndex};
pub use maintenance::{
    CALYX_INDEX_STALE_ENTRY, IndexMaintenance, collection_has_maintained_index, field_is_indexed,
};
pub use rebuild::{IndexHealth, RebuildStats, index_rebuild, index_verify};

use calyx_core::{CalyxError, Result};

use crate::collection::{CALYX_INVALID_ARGUMENT, FieldType, SecondaryIndexKind};
use crate::layers::{RecordKey, RecordValue};

/// A concrete value being indexed. Alias of the relational layer's
/// [`RecordValue`] so index keys and stored rows speak one value vocabulary.
pub type FieldValue = RecordValue;

/// The kind of secondary index. Alias of the collection-declaration
/// [`SecondaryIndexKind`] so a declared index and its runtime form agree on
/// kind without a second enum to keep in sync.
pub type IndexKind = SecondaryIndexKind;

/// Stable, collection-scoped identifier for a secondary index. Encoded as 4
/// big-endian bytes inside every index key so two indexes over the same
/// collection occupy disjoint, ordered keyspaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IndexId(pub u32);

impl IndexId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    /// 4-byte big-endian wire form used in the index-key prefix.
    pub const fn to_be_bytes(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }
}

/// Runtime descriptor for one secondary index: the resolved field type plus a
/// stable id, used to encode and decode that index's keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexSpec {
    /// Stable identifier within the owning collection.
    pub index_id: IndexId,
    /// Human-readable index name (matches the declaring `SecondaryIndexSpec`).
    pub name: String,
    /// Index implementation kind.
    pub kind: IndexKind,
    /// Field name (relational) or document path the index is built over.
    pub on_field: String,
    /// Resolved type of the indexed field — drives the key encoding.
    pub field_type: FieldType,
}

impl IndexSpec {
    pub fn new(
        index_id: IndexId,
        name: impl Into<String>,
        kind: IndexKind,
        on_field: impl Into<String>,
        field_type: FieldType,
    ) -> Self {
        Self {
            index_id,
            name: name.into(),
            kind,
            on_field: on_field.into(),
            field_type,
        }
    }

    /// Fail-closed validation: non-empty name/field. Returns
    /// `CALYX_INVALID_ARGUMENT` on any violation.
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(invalid_index_input("index spec name must be non-empty"));
        }
        if self.on_field.is_empty() {
            return Err(invalid_index_input("index spec on_field must be non-empty"));
        }
        Ok(())
    }
}

/// Every secondary-index implementation maps field values onto the Aster
/// ordered keyspace. Implementations are `Send + Sync` so they can live behind
/// a shared vault handle and be addressed as `dyn SecondaryIndex`.
pub trait SecondaryIndex: Send + Sync {
    /// The index kind this implementation provides.
    fn kind(&self) -> IndexKind;

    /// Encode the full index key for a `(field value, primary key)` pair.
    ///
    /// Fails closed with `CALYX_INVALID_ARGUMENT` if `field_val` does not match
    /// the index's declared field type, or is otherwise unindexable (a NULL, or
    /// a non-finite float). The relational layer never writes non-finite floats,
    /// so this is a structural guard, not an expected path.
    fn encode_index_key(&self, field_val: &FieldValue, pk: &RecordKey) -> Result<Vec<u8>>;

    /// Encode the key prefix that selects every entry with exactly `field_val`
    /// (all primary keys). Same fail-closed contract as [`Self::encode_index_key`].
    fn encode_scan_prefix(&self, field_val: &FieldValue) -> Result<Vec<u8>>;
}

/// Returns the [`FieldType`] a given [`FieldValue`] indexes as, or `None` for a
/// NULL (which carries no type and is not indexable in a btree key).
pub(crate) fn field_value_type(value: &FieldValue) -> Option<FieldType> {
    match value {
        RecordValue::Bool(_) => Some(FieldType::Bool),
        RecordValue::I64(_) => Some(FieldType::I64),
        RecordValue::F64(_) => Some(FieldType::F64),
        RecordValue::Text(_) => Some(FieldType::Text),
        RecordValue::Bytes(_) => Some(FieldType::Bytes),
        RecordValue::Timestamp(_) => Some(FieldType::Timestamp),
        RecordValue::U64(_) => Some(FieldType::U64),
        RecordValue::Null => None,
    }
}

/// Builds a `CALYX_INVALID_ARGUMENT` error for unindexable encode input.
pub(crate) fn invalid_index_input(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_INVALID_ARGUMENT,
        message: message.into(),
        remediation: "index a value matching the index's declared field type",
    }
}
