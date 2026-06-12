use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use calyx_core::{Clock, LedgerRef, LensId, Result};
use calyx_ledger::LedgerCfStore;
use serde::Serialize;

use super::types::{
    CALYX_ANNEAL_PARK_THRESHOLD_NOT_MET, CALYX_ANNEAL_UNPARK_THRESHOLD_NOT_MET, LensParkOutcome,
    SIGNAL_DECAY_FLOOR_BITS, action_label, alert_error, lens_change_id, lens_hash,
    threshold_not_met,
};
use crate::{
    AnnealLedger, AnnealLedgerAction, AnnealLedgerEntry, ComponentHealth, ComponentKind,
    DegradeRegistry, HealthStorage, LogicalTime, MetricSnapshot,
};

pub fn park_decayed_lens<S, L, C>(
    lens_id: LensId,
    bits: f64,
    registry: &mut DegradeRegistry<S>,
    ledger: &mut AnnealLedger<L, C>,
    clock: &dyn Clock,
    alerts_path: &Path,
) -> Result<LensParkOutcome>
where
    S: HealthStorage,
    L: LedgerCfStore,
    C: Clock,
{
    if !bits.is_finite() || bits >= SIGNAL_DECAY_FLOOR_BITS {
        return Err(threshold_not_met(
            CALYX_ANNEAL_PARK_THRESHOLD_NOT_MET,
            bits,
            "park requires bits < 0.05",
        ));
    }
    let kind = ComponentKind::lens_endpoint(lens_id);
    if matches!(registry.health(&kind), ComponentHealth::Parked { .. }) {
        return Ok(LensParkOutcome::AlreadyParked { lens_id });
    }
    let ts = clock.now();
    registry.set_health(
        kind,
        ComponentHealth::parked(ts, format!("signal_decayed bits={bits:.6}")),
        ledger,
    )?;
    write_lens_event(ledger, AnnealLedgerAction::LensPark, lens_id, bits, ts)?;
    append_lens_alert(alerts_path, "lens_park", lens_id, bits, ts)?;
    Ok(LensParkOutcome::Parked { lens_id })
}

pub fn unpark_lens<S, L, C>(
    lens_id: LensId,
    new_bits: f64,
    registry: &mut DegradeRegistry<S>,
    ledger: &mut AnnealLedger<L, C>,
    clock: &dyn Clock,
    alerts_path: &Path,
) -> Result<LensParkOutcome>
where
    S: HealthStorage,
    L: LedgerCfStore,
    C: Clock,
{
    if !new_bits.is_finite() || new_bits < SIGNAL_DECAY_FLOOR_BITS {
        return Err(threshold_not_met(
            CALYX_ANNEAL_UNPARK_THRESHOLD_NOT_MET,
            new_bits,
            "unpark requires bits >= 0.05",
        ));
    }
    let kind = ComponentKind::lens_endpoint(lens_id);
    if matches!(registry.health(&kind), ComponentHealth::Ok) {
        return Ok(LensParkOutcome::AlreadyOk { lens_id });
    }
    let ts = clock.now();
    registry.set_health(kind, ComponentHealth::Ok, ledger)?;
    write_lens_event(
        ledger,
        AnnealLedgerAction::LensUnpark,
        lens_id,
        new_bits,
        ts,
    )?;
    append_lens_alert(alerts_path, "lens_unpark", lens_id, new_bits, ts)?;
    Ok(LensParkOutcome::Unparked { lens_id })
}

fn write_lens_event<L, C>(
    ledger: &mut AnnealLedger<L, C>,
    action: AnnealLedgerAction,
    lens_id: LensId,
    bits: f64,
    ts: LogicalTime,
) -> Result<LedgerRef>
where
    L: LedgerCfStore,
    C: Clock,
{
    ledger.write(AnnealLedgerEntry {
        action,
        change_id: lens_change_id(lens_id, ts, action),
        artifact_id: lens_id.to_string(),
        prior_ptr_hash: lens_hash(lens_id, bits, b"prior"),
        candidate_ptr_hash: lens_hash(lens_id, bits, action_label(action).as_bytes()),
        metrics: MetricSnapshot::empty(ts),
        ts,
        description: format!("{} bits={bits:.6}", action_label(action)),
        fault: None,
        proposal: None,
        details: None,
        prev_hash: None,
    })
}

fn append_lens_alert(
    path: &Path,
    action: &'static str,
    lens_id: LensId,
    bits: f64,
    ts: LogicalTime,
) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| alert_error("alert path has no parent"))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| alert_error(format!("create alert dir: {error}")))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| alert_error(format!("open alerts.jsonl: {error}")))?;
    let line = serde_json::to_vec(&LensAlert {
        tag: "anneal_lens_park_alert_v1",
        action,
        lens_id: lens_id.to_string(),
        bits,
        threshold_bits: SIGNAL_DECAY_FLOOR_BITS,
        ts,
    })
    .map_err(|error| alert_error(format!("encode alert line: {error}")))?;
    file.write_all(&line)
        .and_then(|_| file.write_all(b"\n"))
        .map_err(|error| alert_error(format!("write alerts.jsonl: {error}")))
}

#[derive(Serialize)]
struct LensAlert {
    tag: &'static str,
    action: &'static str,
    lens_id: String,
    bits: f64,
    threshold_bits: f64,
    ts: LogicalTime,
}
