//! The `time_index` column family: a wall-clock → MVCC-seqno map.
//!
//! Each committed group-commit writes one entry whose **key is the data** —
//! `big_endian_u64(millis_utc) || big_endian_u64(seqno)` — and whose value is a
//! single sentinel byte. Big-endian ordering means a forward range scan up to
//! `millis = t` lands the `floor(t)` entry as its last element, so resolving a
//! timestamp to the greatest seqno `≤ t` is a single bounded scan with no WAL
//! replay (PRD `17 §8`). The index is the sole source of truth for the
//! time→seqno mapping.

use calyx_core::{CalyxError, Clock, Result, Seq};

use crate::cf::{ColumnFamily, KeyRange};
use crate::vault::AsterVault;

/// Sentinel value stored under every time-index key (the key carries the data).
pub(crate) const SENTINEL: &[u8] = &[0u8];

const KEY_LEN: usize = 16;

/// Encodes a `(millis, seqno)` time-index key.
pub(crate) fn encode_key(millis: u64, seqno: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(KEY_LEN);
    key.extend_from_slice(&millis.to_be_bytes());
    key.extend_from_slice(&seqno.to_be_bytes());
    key
}

/// Decodes a `(millis, seqno)` time-index key, failing closed on a malformed
/// key rather than returning a silently wrong seqno.
pub(crate) fn decode_key(key: &[u8]) -> Result<(u64, u64)> {
    if key.len() != KEY_LEN {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "time_index key must be {KEY_LEN} bytes, found {}",
            key.len()
        )));
    }
    let millis = u64::from_be_bytes(key[..8].try_into().expect("8-byte millis"));
    let seqno = u64::from_be_bytes(key[8..].try_into().expect("8-byte seqno"));
    Ok((millis, seqno))
}

/// The `(cf, key, value)` triple to append to a group-commit batch for `seqno`
/// committed at `millis`.
pub(crate) fn entry_row(millis: u64, seqno: Seq) -> (ColumnFamily, Vec<u8>, Vec<u8>) {
    (
        ColumnFamily::TimeIndex,
        encode_key(millis, seqno),
        SENTINEL.to_vec(),
    )
}

/// One decoded time-index entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeIndexEntry {
    pub millis: u64,
    pub seqno: Seq,
}

/// Half-open range covering every key with `millis ≤ t_millis`. `t == u64::MAX`
/// yields an unbounded upper end.
fn floor_range(t_millis: u64) -> KeyRange {
    let end = t_millis.checked_add(1).map(|next| encode_key(next, 0));
    KeyRange {
        start: encode_key(0, 0),
        end,
    }
}

/// Resolves `t_millis` to the greatest seqno committed at or before it, reading
/// the index at the vault's latest sequence. Returns `CALYX_TIMETRAVEL_NO_DATA`
/// when the vault has no entry at or before `t` (an explicit empty result, never
/// a silent stale seqno).
pub(crate) fn resolve<C: Clock>(vault: &AsterVault<C>, t_millis: u64) -> Result<Seq> {
    let latest = vault.latest_seq();
    let rows = vault.scan_cf_range_at(latest, ColumnFamily::TimeIndex, &floor_range(t_millis))?;
    let mut resolved = None;
    for (key, _) in rows {
        let (_, seqno) = decode_key(&key)?;
        resolved = Some(seqno);
    }
    resolved.ok_or_else(|| no_data(format!("no time-index entry at or before t={t_millis}ms")))
}

/// Reads every time-index entry visible at the vault's latest sequence, in
/// `(millis, seqno)` order. Used for FSV readback of the source of truth.
pub fn read_all<C: Clock>(vault: &AsterVault<C>) -> Result<Vec<TimeIndexEntry>> {
    let latest = vault.latest_seq();
    vault
        .scan_cf_at(latest, ColumnFamily::TimeIndex)?
        .into_iter()
        .map(|(key, _)| {
            let (millis, seqno) = decode_key(&key)?;
            Ok(TimeIndexEntry { millis, seqno })
        })
        .collect()
}

pub(crate) fn no_data(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_TIMETRAVEL_NO_DATA",
        message: message.into(),
        remediation: "query at or after the first write, or check the vault has any committed data",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_round_trips_and_orders_by_millis_then_seqno() {
        let a = encode_key(1000, 5);
        let b = encode_key(1000, 6);
        let c = encode_key(2000, 1);
        assert!(a < b, "same millis orders by seqno");
        assert!(b < c, "later millis sorts after earlier");
        assert_eq!(decode_key(&a).unwrap(), (1000, 5));
    }

    #[test]
    fn decode_rejects_short_key() {
        assert_eq!(
            decode_key(&[0u8; 8]).unwrap_err().code,
            "CALYX_ASTER_CORRUPT_SHARD"
        );
    }

    #[test]
    fn floor_range_upper_bound_excludes_next_millisecond() {
        let range = floor_range(1500);
        // key at millis=1500, max seqno is still inside the range.
        assert!(range.contains(&encode_key(1500, u64::MAX)));
        // the first key at millis=1501 is excluded.
        assert!(!range.contains(&encode_key(1501, 0)));
    }

    #[test]
    fn floor_range_at_u64_max_is_unbounded() {
        assert!(floor_range(u64::MAX).end.is_none());
    }
}
