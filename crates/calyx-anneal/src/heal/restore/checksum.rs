use calyx_aster::cf::{ColumnFamily, KeyRange};
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, Result, Seq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    BASE_SHARD_CHECKSUM_TAG, BaseFaultEvent, BaseShard, ShardId, decode_hex_vec, decode_hex32, hex,
    invalid_checksum_row,
};

pub const CALYX_ANNEAL_CHECKSUM_INVALID_ROW: &str = "CALYX_ANNEAL_CHECKSUM_INVALID_ROW";

#[derive(Serialize, Deserialize)]
struct ChecksumRow {
    tag: String,
    shard_id: String,
    range_start_hex: String,
    range_end_hex: Option<String>,
    checksum_sha256: String,
    barrier_installed: bool,
    last_actual_sha256: Option<String>,
    updated_at: u64,
}

pub fn record_base_shard_checksum<C>(
    vault: &AsterVault<C>,
    shard: &BaseShard,
    clock: &dyn Clock,
) -> Result<Seq>
where
    C: Clock,
{
    write_shard_status(vault, shard, false, None, clock.now())
}

pub fn load_base_shards<C>(vault: &AsterVault<C>) -> Result<Vec<BaseShard>>
where
    C: Clock,
{
    let mut shards = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealChecksums)?
        .into_iter()
        .map(|(_, value)| decode_row(&value).map(|stored| stored.shard))
        .collect::<Result<Vec<_>>>()?;
    shards.sort_by(|left, right| left.shard_id.cmp(&right.shard_id));
    Ok(shards)
}

pub fn verify_base_shards<C>(
    vault: &AsterVault<C>,
    clock: &dyn Clock,
) -> Result<Vec<BaseFaultEvent>>
where
    C: Clock,
{
    let mut events = Vec::new();
    for shard in load_base_shards(vault)? {
        let actual = base_shard_checksum(vault, &shard.cf_range)?;
        if actual != shard.checksum {
            events.push(BaseFaultEvent::corrupt(shard, actual, clock.now()));
        }
    }
    Ok(events)
}

pub fn base_shard_checksum<C>(vault: &AsterVault<C>, range: &KeyRange) -> Result<[u8; 32]>
where
    C: Clock,
{
    let mut rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Base)?
        .into_iter()
        .filter(|(key, _)| range.contains(key))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(hash_rows(rows))
}

pub(super) fn write_shard_status<C>(
    vault: &AsterVault<C>,
    shard: &BaseShard,
    barrier_installed: bool,
    last_actual: Option<[u8; 32]>,
    updated_at: u64,
) -> Result<Seq>
where
    C: Clock,
{
    let row = ChecksumRow {
        tag: BASE_SHARD_CHECKSUM_TAG.to_string(),
        shard_id: shard.shard_id.to_string(),
        range_start_hex: hex(&shard.cf_range.start),
        range_end_hex: shard.cf_range.end.as_ref().map(|end| hex(end)),
        checksum_sha256: hex(&shard.checksum),
        barrier_installed,
        last_actual_sha256: last_actual.as_ref().map(|actual| hex(actual)),
        updated_at,
    };
    let value = serde_json::to_vec(&row)
        .map_err(|error| invalid_checksum_row(format!("encode checksum row: {error}")))?;
    vault.write_cf(
        ColumnFamily::AnnealChecksums,
        checksum_key(&shard.shard_id),
        value,
    )
}

pub(super) fn load_barriered_shards<C>(vault: &AsterVault<C>) -> Result<Vec<BaseShard>>
where
    C: Clock,
{
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealChecksums)?
        .into_iter()
        .map(|(_, value)| decode_row(&value))
        .filter_map(|stored| match stored {
            Ok(row) if row.barrier_installed => Some(Ok(row.shard)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

struct StoredShard {
    shard: BaseShard,
    barrier_installed: bool,
}

fn decode_row(value: &[u8]) -> Result<StoredShard> {
    let row = serde_json::from_slice::<ChecksumRow>(value)
        .map_err(|error| invalid_checksum_row(format!("decode checksum row: {error}")))?;
    if row.tag != BASE_SHARD_CHECKSUM_TAG {
        return Err(invalid_checksum_row("checksum row has invalid tag"));
    }
    let range = KeyRange {
        start: decode_hex_vec(&row.range_start_hex, "range_start_hex")?,
        end: row
            .range_end_hex
            .as_deref()
            .map(|value| decode_hex_vec(value, "range_end_hex"))
            .transpose()?,
    };
    Ok(StoredShard {
        shard: BaseShard::new(
            ShardId::new(row.shard_id),
            range,
            decode_hex32(&row.checksum_sha256, "checksum_sha256")?,
        ),
        barrier_installed: row.barrier_installed,
    })
}

fn checksum_key(shard_id: &ShardId) -> Vec<u8> {
    format!("base_shard/{}", shard_id.as_str()).into_bytes()
}

fn hash_rows(rows: Vec<(Vec<u8>, Vec<u8>)>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for (key, value) in rows {
        hasher.update((key.len() as u64).to_be_bytes());
        hasher.update(&key);
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(&value);
    }
    hasher.finalize().into()
}
