use crate::cf::{ColumnFamily, base_key};
use crate::recurrence::FREQUENCY_SCALAR;
use crate::vault::{AsterVault, encode};
use calyx_core::{CalyxError, Clock, CxId, Result};
use serde::{Deserialize, Serialize};

use super::dedup_error;

pub const CALYX_DEDUP_MISSING_FREQUENCY: &str = "CALYX_DEDUP_MISSING_FREQUENCY";
pub const CALYX_DEDUP_INVALID_FREQUENCY: &str = "CALYX_DEDUP_INVALID_FREQUENCY";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Domain {
    pub cx_ids: Vec<CxId>,
}

impl Domain {
    pub fn new(cx_ids: Vec<CxId>) -> Self {
        Self { cx_ids }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompressionRatio {
    pub cx_id: CxId,
    pub original_count: u64,
    pub stored_count: u64,
    pub ratio: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainCompressionStats {
    pub total_original: u64,
    pub total_stored: u64,
    pub mean_ratio: f32,
    pub max_ratio: f32,
}

pub fn compression_ratio<C>(cx_id: CxId, vault: &AsterVault<C>) -> Result<CompressionRatio>
where
    C: Clock,
{
    let frequency = read_frequency(vault, cx_id)?;
    Ok(CompressionRatio {
        cx_id,
        original_count: frequency,
        stored_count: 1,
        ratio: ratio_for_frequency(frequency),
    })
}

pub fn domain_compression_stats<C>(
    domain: &Domain,
    vault: &AsterVault<C>,
) -> Result<DomainCompressionStats>
where
    C: Clock,
{
    let mut total_original = 0_u64;
    let mut max_ratio = 0.0_f32;
    for cx_id in &domain.cx_ids {
        let ratio = compression_ratio(*cx_id, vault)?;
        total_original = total_original
            .checked_add(ratio.original_count)
            .ok_or_else(|| CalyxError::aster_corrupt_shard("compression total overflow"))?;
        max_ratio = max_ratio.max(ratio.ratio);
    }
    let total_stored = domain.cx_ids.len() as u64;
    let mean_ratio = if total_stored == 0 {
        0.0
    } else {
        total_original as f32 / total_stored as f32
    };
    Ok(DomainCompressionStats {
        total_original,
        total_stored,
        mean_ratio,
        max_ratio,
    })
}

fn ratio_for_frequency(frequency: u64) -> f32 {
    if frequency <= 1 {
        1.0
    } else {
        frequency as f32
    }
}

fn read_frequency<C>(vault: &AsterVault<C>, cx_id: CxId) -> Result<u64>
where
    C: Clock,
{
    let bytes = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(cx_id))?
        .ok_or_else(|| {
            dedup_error(
                CALYX_DEDUP_MISSING_FREQUENCY,
                format!("base CF row missing for {cx_id}"),
            )
        })?;
    let cx = encode::decode_constellation_base(&bytes)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("decode base CF: {error}")))?;
    let value = cx.scalars.get(FREQUENCY_SCALAR).ok_or_else(|| {
        dedup_error(
            CALYX_DEDUP_MISSING_FREQUENCY,
            format!("{FREQUENCY_SCALAR} missing for {cx_id}"),
        )
    })?;
    if !value.is_finite() || *value < 0.0 || value.fract() != 0.0 || *value > u64::MAX as f64 {
        return Err(dedup_error(
            CALYX_DEDUP_INVALID_FREQUENCY,
            format!("{FREQUENCY_SCALAR} for {cx_id} must be a non-negative integer"),
        ));
    }
    Ok(*value as u64)
}
