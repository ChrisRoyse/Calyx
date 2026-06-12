use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use calyx_core::{Clock, LedgerRef, Result};
use calyx_ledger::LedgerCfStore;
use serde::Serialize;

use crate::{AnnealLedger, AnnealLedgerAction, AnnealLedgerEntry, ChangeId, MetricSnapshot};

use super::{BaseFaultEvent, BaseShard, CALYX_ANNEAL_ALERT_WRITE_FAILED, hex};

pub fn alert_operator<L, C>(
    event: &BaseFaultEvent,
    ledger: &mut AnnealLedger<L, C>,
    alerts_path: &Path,
) -> Result<LedgerRef>
where
    L: LedgerCfStore,
    C: Clock,
{
    let ledger_ref = write_base_corrupt_alert(event, ledger)?;
    append_alert_line(event, alerts_path)?;
    Ok(ledger_ref)
}

pub fn write_base_restored_event<L, C>(
    shard: &BaseShard,
    ledger: &mut AnnealLedger<L, C>,
    ts: u64,
) -> Result<LedgerRef>
where
    L: LedgerCfStore,
    C: Clock,
{
    ledger.write(AnnealLedgerEntry {
        action: AnnealLedgerAction::BaseRestored,
        change_id: change_id(shard.shard_id.as_str(), ts),
        artifact_id: artifact_id(shard),
        prior_ptr_hash: shard.checksum,
        candidate_ptr_hash: shard.checksum,
        metrics: MetricSnapshot::empty(ts),
        ts,
        description: format!("base shard restored shard_id={}", shard.shard_id),
        fault: None,
        proposal: None,
        details: None,
        prev_hash: None,
    })
}

fn write_base_corrupt_alert<L, C>(
    event: &BaseFaultEvent,
    ledger: &mut AnnealLedger<L, C>,
) -> Result<LedgerRef>
where
    L: LedgerCfStore,
    C: Clock,
{
    let shard = event.shard();
    ledger.write(AnnealLedgerEntry {
        action: AnnealLedgerAction::BaseCorruptAlert,
        change_id: change_id(shard.shard_id.as_str(), event.detected_at()),
        artifact_id: artifact_id(shard),
        prior_ptr_hash: event.expected(),
        candidate_ptr_hash: event.actual(),
        metrics: MetricSnapshot::empty(event.detected_at()),
        ts: event.detected_at(),
        description: format!("base corrupt alert shard_id={}", shard.shard_id),
        fault: None,
        proposal: None,
        details: None,
        prev_hash: None,
    })
}

fn append_alert_line(event: &BaseFaultEvent, path: &Path) -> Result<()> {
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
    let line = serde_json::to_vec(&AlertLine::from(event))
        .map_err(|error| alert_error(format!("encode alert line: {error}")))?;
    file.write_all(&line)
        .and_then(|_| file.write_all(b"\n"))
        .map_err(|error| alert_error(format!("write alerts.jsonl: {error}")))
}

#[derive(Serialize)]
struct AlertLine {
    tag: &'static str,
    action: &'static str,
    shard_id: String,
    range_start_hex: String,
    range_end_hex: Option<String>,
    expected_sha256: String,
    actual_sha256: String,
    detected_at: u64,
}

impl From<&BaseFaultEvent> for AlertLine {
    fn from(event: &BaseFaultEvent) -> Self {
        let shard = event.shard();
        Self {
            tag: "anneal_base_corrupt_alert_v1",
            action: "base_corrupt_alert",
            shard_id: shard.shard_id.to_string(),
            range_start_hex: hex(&shard.cf_range.start),
            range_end_hex: shard.cf_range.end.as_ref().map(|end| hex(end)),
            expected_sha256: hex(&event.expected()),
            actual_sha256: hex(&event.actual()),
            detected_at: event.detected_at(),
        }
    }
}

fn change_id(shard_id: &str, ts: u64) -> ChangeId {
    let mut acc = ts ^ 0x9e37_79b9_7f4a_7c15;
    for byte in shard_id.bytes() {
        acc ^= u64::from(byte);
        acc = acc.wrapping_mul(0x1000_0000_01b3);
    }
    ChangeId(acc.max(1))
}

fn artifact_id(shard: &BaseShard) -> String {
    format!("base_shard/{}", shard.shard_id)
}

fn alert_error(message: impl Into<String>) -> calyx_core::CalyxError {
    calyx_core::CalyxError {
        code: CALYX_ANNEAL_ALERT_WRITE_FAILED,
        message: message.into(),
        remediation: "repair the vault alert path; ledger alert was already attempted",
    }
}
