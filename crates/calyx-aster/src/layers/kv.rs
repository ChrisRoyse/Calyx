//! KV `(ns, key) -> value` layer with check-on-read TTL.
//!
//! KV state is O(1) keyed storage scoped per collection and namespace, sitting
//! on the same ordered transactional core as the relational and document
//! layers and addressed by the disjoint `0x03` key-space discriminant.
//!
//! TTL is enforced **check-on-read** for PH53: an expired record is invisible
//! to [`KvLayer::kv_get`] even though its bytes remain on disk until the PH58
//! background janitor physically reclaims them. Deletion writes the native
//! MVCC tombstone, which the read path filters, so a deleted key reads back as
//! absent.

use std::time::Duration;

use calyx_core::{CalyxError, Clock, Modality, Result, Seq};

use crate::cf::{ColumnFamily, KeyRange, prefix_range};
use crate::collection::{
    CALYX_INVALID_ARGUMENT, Collection, CollectionMode, FieldType, Schema, collection_has_lens,
    ingest_collection_constellation,
};
use crate::index::{IndexMaintenance, collection_has_maintained_index, field_is_indexed};
use crate::layers::relational::{RecordKey, RecordValue, Row};
use crate::mvcc::tombstone_value;
use crate::vault::AsterVault;
use calyx_ledger::{ActorId, EntryKind, PayloadBuilder, RedactionPolicy, SubjectId};

use super::Layer;

/// Key-space discriminant for KV rows.
const DISC_KV: u8 = 0x03;
/// On-disk value layout version. A non-zero leading byte also guarantees a
/// live KV value can never alias the MVCC tombstone sentinel (which begins
/// with `0x00`), so `kv_get` cannot mistake a stored value for a deletion.
const KV_VALUE_VERSION: u8 = 0x01;
/// `version (1) || expires_at_ms (u64 BE)` header preceding the payload.
const VALUE_HEADER_BYTES: usize = 1 + 8;
const MAX_USER_KEY_BYTES: usize = u16::MAX as usize;
const MAX_PAYLOAD_BYTES: usize = 1 << 20;

/// `(ns, key) -> value` key-encoding layer over a `KV` collection.
pub struct KvLayer<'a, C: Clock> {
    vault: &'a AsterVault<C>,
}

impl<'a, C: Clock> KvLayer<'a, C> {
    pub fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }

    /// Writes `val` at `(ns, key)`, optionally expiring it `ttl` after the
    /// current server clock. `ttl == None` stores a non-expiring record.
    pub fn kv_set(
        &self,
        col: &Collection,
        ns: u64,
        key: &[u8],
        val: &[u8],
        ttl: Option<Duration>,
    ) -> Result<Seq> {
        if collection_has_lens(col) {
            validate_user_key(key)?;
            validate_payload(val)?;
            let expires_at = self.expires_at(ttl)?;
            let full_key = kv_key(col, ns, key);
            let value = encode_value(expires_at, val);
            let parts = [
                ("kv_key", full_key.as_slice()),
                ("kv_value", value.as_slice()),
            ];
            return ingest_collection_constellation(
                self.vault,
                col,
                "kv",
                &parts,
                Modality::Structured,
            );
        }
        require_kv_mode(col)?;
        validate_user_key(key)?;
        validate_payload(val)?;
        let expires_at = self.expires_at(ttl)?;
        let full_key = kv_key(col, ns, key);
        let value = encode_value(expires_at, val);
        let pk = RecordKey::from_bytes(full_key.clone())?;
        let mut rows = vec![(ColumnFamily::Kv, full_key.clone(), value.clone())];
        // Only touch secondary indexes — and the extra read they require — when
        // the collection actually declares one. Unindexed KV writes stay on the
        // O(1) key path and never coerce synthetic index fields.
        if collection_has_maintained_index(col) {
            let old_index_row = self
                .kv_get(col, ns, key)?
                .map(|old| kv_index_row(col, ns, key, &old))
                .transpose()?;
            let new_index_row = kv_index_row(col, ns, key, val)?;
            IndexMaintenance::stage_put(
                self.vault,
                &mut rows,
                col,
                &pk,
                old_index_row.as_ref(),
                &new_index_row,
            )?;
        }
        let subject = ledger_subject(&full_key);
        let payload = ledger_payload(col, ns, &full_key, &value);
        self.vault.write_cf_batch_with_ledger_entry(
            rows,
            EntryKind::Ingest,
            subject,
            payload,
            ActorId::Service("calyx-aster-kv".to_string()),
        )
    }

    /// Reads the live value at `(ns, key)`. Returns `None` if the key is
    /// absent, tombstoned, or expired. Never returns expired bytes.
    pub fn kv_get(&self, col: &Collection, ns: u64, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.kv_get_at(self.vault.latest_seq(), col, ns, key)
    }

    /// Snapshot-pinned variant of [`Self::kv_get`].
    pub fn kv_get_at(
        &self,
        snapshot: Seq,
        col: &Collection,
        ns: u64,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        require_kv_mode(col)?;
        validate_user_key(key)?;
        let full_key = kv_key(col, ns, key);
        let Some(bytes) = self
            .vault
            .read_cf_at(snapshot, ColumnFamily::Kv, &full_key)?
        else {
            return Ok(None);
        };
        let (expires_at, payload) = decode_value(&bytes)?;
        if is_expired(expires_at, self.vault.clock_now()) {
            return Ok(None);
        }
        Ok(Some(payload.to_vec()))
    }

    /// Deletes `(ns, key)` by writing the native MVCC tombstone. A subsequent
    /// `kv_get` reads back as absent; the tombstone is reclaimed by the PH58
    /// janitor.
    pub fn kv_delete(&self, col: &Collection, ns: u64, key: &[u8]) -> Result<Seq> {
        require_kv_mode(col)?;
        validate_user_key(key)?;
        let full_key = kv_key(col, ns, key);
        let value = tombstone_value();
        let pk = RecordKey::from_bytes(full_key.clone())?;
        let mut rows = vec![(ColumnFamily::Kv, full_key.clone(), value.clone())];
        // See `kv_set`: skip the index read+stage entirely when no index exists.
        if collection_has_maintained_index(col) {
            let old_index_row = self
                .kv_get(col, ns, key)?
                .map(|old| kv_index_row(col, ns, key, &old))
                .transpose()?;
            if let Some(old_index_row) = &old_index_row {
                IndexMaintenance::stage_delete(self.vault, &mut rows, col, &pk, old_index_row)?;
            }
        }
        let subject = ledger_subject(&full_key);
        let payload = ledger_payload(col, ns, &full_key, &value);
        self.vault.write_cf_batch_with_ledger_entry(
            rows,
            EntryKind::Ingest,
            subject,
            payload,
            ActorId::Service("calyx-aster-kv".to_string()),
        )
    }

    /// Returns live `(user_key, payload)` pairs in namespace `ns` whose user
    /// keys fall in `[start, end)`, ordered by user key, capped at `limit`.
    ///
    /// KV keys are length-prefixed (so on-disk order is length-major); this
    /// scans the whole namespace prefix and re-sorts by user key, giving
    /// lexicographic results independent of stored encoding.
    pub fn kv_range(
        &self,
        col: &Collection,
        ns: u64,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        require_kv_mode(col)?;
        if limit == 0 || start >= end {
            return Ok(Vec::new());
        }
        let now = self.vault.clock_now();
        let rows = self.vault.scan_cf_range_at(
            self.vault.latest_seq(),
            ColumnFamily::Kv,
            &namespace_range(col, ns),
        )?;
        let mut out = Vec::new();
        for (full_key, value) in rows {
            let user_key = decode_user_key(col, ns, &full_key)?;
            if user_key.as_slice() < start || user_key.as_slice() >= end {
                continue;
            }
            let (expires_at, payload) = decode_value(&value)?;
            if is_expired(expires_at, now) {
                continue;
            }
            out.push((user_key, payload.to_vec()));
        }
        out.sort_by(|left, right| left.0.cmp(&right.0));
        out.truncate(limit);
        Ok(out)
    }

    fn expires_at(&self, ttl: Option<Duration>) -> Result<u64> {
        match ttl {
            None => Ok(0),
            Some(duration) => {
                let now = self.vault.clock_now();
                let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
                if millis == 0 {
                    return Err(invalid_argument(
                        "kv ttl must be >= 1ms (got a sub-millisecond duration)",
                    ));
                }
                Ok(now.saturating_add(millis))
            }
        }
    }
}

impl<C> Layer for KvLayer<'_, C>
where
    C: Clock + Send + Sync,
{
    fn collection_mode() -> CollectionMode {
        CollectionMode::KV
    }

    fn put(&self, col: &Collection, key: &[u8], value: &[u8]) -> Result<()> {
        self.kv_set(col, 0, key, value, None)?;
        Ok(())
    }

    fn get(&self, col: &Collection, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.kv_get(col, 0, key)
    }

    fn range(
        &self,
        col: &Collection,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<Vec<u8>>> {
        Ok(self
            .kv_range(col, 0, start, end, limit)?
            .into_iter()
            .map(|(_, value)| value)
            .collect())
    }
}

/// Stable per-collection id used to scope KV rows. Distinct hash domain from
/// the other layers so collisions across modes are impossible.
pub fn collection_id(col: &Collection) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"calyx:kv:collection:v1");
    hasher.update(&col.tenant.0.to_be_bytes());
    hasher.update(&(col.name.len() as u16).to_be_bytes());
    hasher.update(col.name.as_bytes());
    u64::from_be_bytes(hasher.finalize().as_bytes()[0..8].try_into().unwrap())
}

/// Encodes the full KV row key: `0x03 | cid | ns | key_len | user_key`.
pub fn kv_key(col: &Collection, ns: u64, user_key: &[u8]) -> Vec<u8> {
    let mut key = namespace_prefix(col, ns);
    key.extend_from_slice(&(user_key.len() as u16).to_be_bytes());
    key.extend_from_slice(user_key);
    key
}

fn namespace_prefix(col: &Collection, ns: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(1 + 8 + 8 + 2);
    key.push(DISC_KV);
    key.extend_from_slice(&collection_id(col).to_be_bytes());
    key.extend_from_slice(&ns.to_be_bytes());
    key
}

fn namespace_range(col: &Collection, ns: u64) -> KeyRange {
    prefix_range(&namespace_prefix(col, ns))
}

fn decode_user_key(col: &Collection, ns: u64, full_key: &[u8]) -> Result<Vec<u8>> {
    let prefix = namespace_prefix(col, ns);
    let rest = full_key
        .strip_prefix(prefix.as_slice())
        .ok_or_else(|| corrupt("kv scan returned a key outside the namespace prefix"))?;
    let len_bytes = rest
        .get(0..2)
        .ok_or_else(|| corrupt("kv key is missing its user-key length prefix"))?;
    let len = u16::from_be_bytes([len_bytes[0], len_bytes[1]]) as usize;
    let user_key = rest
        .get(2..2 + len)
        .ok_or_else(|| corrupt("kv key length prefix exceeds the stored key"))?;
    if 2 + len != rest.len() {
        return Err(corrupt("kv key has trailing bytes after the user key"));
    }
    Ok(user_key.to_vec())
}

/// Builds the synthetic index row for a KV record, carrying only the well-known
/// fields (`ns`, `key`, `value`) that a maintained index actually references.
///
/// `ns` is a full `u64`; schema-less namespace indexes use native `U64`, while
/// explicit schema types keep their declared encoding and fail closed when they
/// cannot represent the namespace.
pub(crate) fn kv_index_row(col: &Collection, ns: u64, key: &[u8], val: &[u8]) -> Result<Row> {
    let mut fields = Vec::new();
    if field_is_indexed(col, "ns") {
        fields.push(("ns", kv_namespace_value(col, ns)?));
    }
    if field_is_indexed(col, "key") {
        fields.push(("key", RecordValue::Bytes(key.to_vec())));
    }
    if field_is_indexed(col, "value") {
        fields.push(("value", RecordValue::Bytes(val.to_vec())));
    }
    Ok(Row::new(fields))
}

fn kv_namespace_value(col: &Collection, ns: u64) -> Result<RecordValue> {
    match declared_field_type(col, "ns") {
        Some(FieldType::I64) => i64::try_from(ns)
            .map(RecordValue::I64)
            .map_err(|_| invalid_argument("kv namespace exceeds i64 indexable range")),
        Some(FieldType::U64) | None => Ok(RecordValue::U64(ns)),
        Some(FieldType::Timestamp) => i64::try_from(ns)
            .map(RecordValue::Timestamp)
            .map_err(|_| invalid_argument("kv namespace exceeds timestamp indexable range")),
        Some(FieldType::Text) => Ok(RecordValue::Text(ns.to_string())),
        Some(FieldType::Bytes) => Ok(RecordValue::Bytes(ns.to_be_bytes().to_vec())),
        Some(FieldType::Bool) | Some(FieldType::F64) => Err(invalid_argument(
            "kv namespace index field must be U64, Bytes, Text, I64, or Timestamp",
        )),
    }
}

fn declared_field_type(col: &Collection, field: &str) -> Option<FieldType> {
    let Some(Schema::SchemaFull(fields)) = &col.schema else {
        return None;
    };
    fields
        .iter()
        .find(|declared| declared.name == field)
        .map(|declared| declared.ty)
}

pub(crate) fn encode_value(expires_at: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(VALUE_HEADER_BYTES + payload.len());
    out.push(KV_VALUE_VERSION);
    out.extend_from_slice(&expires_at.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Returns `(expires_at_ms, payload)` from a stored value, failing closed on a
/// short or wrong-version row.
pub(crate) fn decode_value(bytes: &[u8]) -> Result<(u64, &[u8])> {
    if bytes.len() < VALUE_HEADER_BYTES {
        return Err(corrupt("kv value is shorter than its header"));
    }
    if bytes[0] != KV_VALUE_VERSION {
        return Err(corrupt(format!(
            "unsupported kv value version {} (expected {KV_VALUE_VERSION})",
            bytes[0]
        )));
    }
    let expires_at = u64::from_be_bytes(bytes[1..9].try_into().unwrap());
    Ok((expires_at, &bytes[VALUE_HEADER_BYTES..]))
}

pub(crate) fn is_expired(expires_at: u64, now: u64) -> bool {
    expires_at != 0 && now >= expires_at
}

fn require_kv_mode(col: &Collection) -> Result<()> {
    if col.mode == CollectionMode::KV {
        Ok(())
    } else {
        Err(invalid_argument(format!(
            "kv layer requires a KV collection, got {:?}",
            col.mode
        )))
    }
}

pub(crate) fn validate_user_key(key: &[u8]) -> Result<()> {
    if key.is_empty() || key.len() > MAX_USER_KEY_BYTES {
        return Err(invalid_argument(format!(
            "kv user key must be 1..={MAX_USER_KEY_BYTES} bytes"
        )));
    }
    Ok(())
}

pub(crate) fn validate_payload(val: &[u8]) -> Result<()> {
    if val.len() > MAX_PAYLOAD_BYTES {
        return Err(invalid_argument(format!(
            "kv value must be <= {MAX_PAYLOAD_BYTES} bytes"
        )));
    }
    Ok(())
}

fn ledger_subject(full_key: &[u8]) -> SubjectId {
    SubjectId::Query(blake3::hash(full_key).as_bytes().to_vec())
}

fn ledger_payload(col: &Collection, ns: u64, full_key: &[u8], value: &[u8]) -> Vec<u8> {
    let mut payload = PayloadBuilder::default();
    payload
        .insert_str("collection_id", format!("{:016x}", collection_id(col)))
        .insert_str("ns", ns.to_string())
        .insert_str("key_hash", blake3::hash(full_key).to_hex().to_string())
        .insert_str("value_hash", blake3::hash(value).to_hex().to_string());
    RedactionPolicy::default().apply_to_payload(&payload)
}

fn invalid_argument(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_INVALID_ARGUMENT,
        message: message.into(),
        remediation: "fix the kv input",
    }
}

fn corrupt(message: impl Into<String>) -> CalyxError {
    CalyxError::aster_corrupt_shard(message)
}

#[cfg(test)]
mod tests;
