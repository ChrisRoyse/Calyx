use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::RecurrenceSeries;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, CxId, Result};
use serde::{Deserialize, Serialize};

use crate::error::{CALYX_LOOM_SERIES_READ_ERROR, CALYX_LOOM_TEMPORAL_XTERM_CORRUPT, loom_error};

use super::SeriesStore;

const MAGIC: &[u8; 5] = b"LLAG1";
const CX_BYTES: usize = 16;
const F64_BYTES: usize = 8;
const U64_BYTES: usize = 8;
const VALUE_LEN: usize = MAGIC.len() + (CX_BYTES * 2) + F64_BYTES + U64_BYTES + U64_BYTES;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeadLagResult {
    pub cx_a: CxId,
    pub cx_b: CxId,
    pub lead_lag_secs: f64,
    pub n_pairs: usize,
    pub proximity_window_secs: u64,
}

pub fn co_occurrence_pairs(
    series_a: &RecurrenceSeries,
    series_b: &RecurrenceSeries,
    window_secs: u64,
) -> Vec<(EpochSecs, EpochSecs)> {
    if window_secs == 0 {
        return Vec::new();
    }
    let window = i128::from(window_secs);
    let mut pairs = Vec::new();
    for occurrence_a in &series_a.occurrences {
        for occurrence_b in &series_b.occurrences {
            let delta = i128::from(occurrence_b.t_k.0) - i128::from(occurrence_a.t_k.0);
            if delta.abs() < window {
                pairs.push((occurrence_a.t_k, occurrence_b.t_k));
            }
        }
    }
    pairs
}

pub fn lead_lag_secs(
    series_a: &RecurrenceSeries,
    series_b: &RecurrenceSeries,
    window_secs: u64,
) -> Option<LeadLagResult> {
    if window_secs == 0 {
        return None;
    }
    if series_a.cx_id == series_b.cx_id {
        let n_pairs = series_a.occurrences.len();
        return (n_pairs >= 3).then_some(LeadLagResult {
            cx_a: series_a.cx_id,
            cx_b: series_b.cx_id,
            lead_lag_secs: 0.0,
            n_pairs,
            proximity_window_secs: window_secs,
        });
    }

    let pairs = co_occurrence_pairs(series_a, series_b, window_secs);
    if pairs.len() < 3 {
        return None;
    }
    let mut deltas = pairs
        .iter()
        .map(|(t_a, t_b)| (t_b.0 as f64) - (t_a.0 as f64))
        .collect::<Vec<_>>();
    deltas.sort_by(f64::total_cmp);
    let mid = deltas.len() / 2;
    let lead_lag = if deltas.len() % 2 == 0 {
        (deltas[mid - 1] + deltas[mid]) / 2.0
    } else {
        deltas[mid]
    };
    Some(LeadLagResult {
        cx_a: series_a.cx_id,
        cx_b: series_b.cx_id,
        lead_lag_secs: lead_lag,
        n_pairs: pairs.len(),
        proximity_window_secs: window_secs,
    })
}

pub fn temporal_cross_term<C>(
    cx_a: CxId,
    cx_b: CxId,
    vault: &AsterVault<C>,
    window_secs: u64,
) -> Result<Option<LeadLagResult>>
where
    C: Clock,
{
    let store = SeriesStore::new(vault);
    let series_a = store
        .read_series(cx_a)
        .map_err(|error| series_read_error(cx_a, error))?;
    let series_b = if cx_a == cx_b {
        series_a.clone()
    } else {
        store
            .read_series(cx_b)
            .map_err(|error| series_read_error(cx_b, error))?
    };
    let result = lead_lag_secs(&series_a, &series_b, window_secs);
    if cx_a == cx_b {
        return Ok(result);
    }
    if let Some(result) = &result {
        vault.put_temporal_xterm(cx_a, cx_b, encode_lead_lag_result(result)?)?;
    }
    Ok(result)
}

pub fn encode_lead_lag_result(result: &LeadLagResult) -> Result<Vec<u8>> {
    if !result.lead_lag_secs.is_finite() {
        return Err(temporal_xterm_corrupt("lead_lag_secs must be finite"));
    }
    let n_pairs = u64::try_from(result.n_pairs)
        .map_err(|_| temporal_xterm_corrupt("n_pairs does not fit u64"))?;
    let mut out = Vec::with_capacity(VALUE_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(result.cx_a.as_bytes());
    out.extend_from_slice(result.cx_b.as_bytes());
    out.extend_from_slice(&result.lead_lag_secs.to_be_bytes());
    out.extend_from_slice(&n_pairs.to_be_bytes());
    out.extend_from_slice(&result.proximity_window_secs.to_be_bytes());
    Ok(out)
}

pub fn decode_lead_lag_result(bytes: &[u8]) -> Result<LeadLagResult> {
    if bytes.len() != VALUE_LEN {
        return Err(temporal_xterm_corrupt(format!(
            "temporal_xterm value is {} bytes; expected {VALUE_LEN}",
            bytes.len()
        )));
    }
    if &bytes[..MAGIC.len()] != MAGIC {
        return Err(temporal_xterm_corrupt("temporal_xterm magic mismatch"));
    }
    let mut offset = MAGIC.len();
    let cx_a = read_cx_id(bytes, &mut offset);
    let cx_b = read_cx_id(bytes, &mut offset);
    let lead_lag_secs = f64::from_be_bytes(read_array(bytes, &mut offset));
    if !lead_lag_secs.is_finite() {
        return Err(temporal_xterm_corrupt("lead_lag_secs must be finite"));
    }
    let n_pairs = usize::try_from(u64::from_be_bytes(read_array(bytes, &mut offset)))
        .map_err(|_| temporal_xterm_corrupt("n_pairs does not fit usize"))?;
    let proximity_window_secs = u64::from_be_bytes(read_array(bytes, &mut offset));
    Ok(LeadLagResult {
        cx_a,
        cx_b,
        lead_lag_secs,
        n_pairs,
        proximity_window_secs,
    })
}

fn read_cx_id(bytes: &[u8], offset: &mut usize) -> CxId {
    CxId::from_bytes(read_array(bytes, offset))
}

fn read_array<const N: usize>(bytes: &[u8], offset: &mut usize) -> [u8; N] {
    let mut out = [0_u8; N];
    out.copy_from_slice(&bytes[*offset..*offset + N]);
    *offset += N;
    out
}

fn series_read_error(cx_id: CxId, error: CalyxError) -> CalyxError {
    loom_error(
        CALYX_LOOM_SERIES_READ_ERROR,
        format!(
            "read recurrence series for {cx_id} failed with {}: {}",
            error.code, error.message
        ),
    )
}

fn temporal_xterm_corrupt(message: impl Into<String>) -> CalyxError {
    loom_error(CALYX_LOOM_TEMPORAL_XTERM_CORRUPT, message)
}
