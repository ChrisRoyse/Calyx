use std::ops::Range;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::manifest::{ManifestStore, QuarantineRecord, is_vault_seq_quarantined};
use calyx_core::CalyxError;
use calyx_ledger::{DirectoryLedgerStore, LedgerCfStore, VerifyResult, verify_chain};

use crate::cf_read::hex_bytes;
use crate::ledger_store::AsterLedgerCfStore;
use crate::merkle::parse_range;

pub fn verify_ledger_dir(ledger: &Path, range: Range<u64>) -> crate::error::CliResult {
    let store = DirectoryLedgerStore::open(ledger)?;
    print_verify_result(verify_chain(&store, range)?)
}

pub fn verify_vault(vault: &Path, range: Range<u64>) -> crate::error::CliResult {
    let store = AsterLedgerCfStore::open(vault)?;
    let result = verify_chain(&store, range.clone())?;
    if let Some(at_seq) = result.quarantine_seq() {
        write_quarantine(vault, range, at_seq)?;
    }
    print_verify_result(result)
}

pub fn readback_ledger_seq(vault: &Path, seq: u64) -> crate::error::CliResult {
    if is_vault_seq_quarantined(vault, seq)? {
        return Err(
            CalyxError::ledger_chain_broken(format!("ledger seq {seq} is quarantined")).into(),
        );
    }
    let store = AsterLedgerCfStore::open(vault)?;
    let rows = store.scan()?;
    let row = rows
        .into_iter()
        .find(|row| row.seq == seq)
        .ok_or_else(|| CalyxError::ledger_corrupt(format!("missing ledger row for seq {seq}")))?;
    println!(
        "CF\tledger\tSEQ\t{}\tKEY\t{}\tVALUE\t{}",
        row.seq,
        hex_bytes(&row.seq.to_be_bytes()),
        hex_bytes(&row.bytes)
    );
    Ok(())
}

pub fn parse_seq(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|error| format!("invalid --seq: {error}"))
}

pub fn parse_verify_range(value: &str) -> Result<Range<u64>, String> {
    parse_range(value)
}

fn write_quarantine(
    vault: &Path,
    range: Range<u64>,
    at_seq: u64,
) -> std::result::Result<(), String> {
    let detected_at_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock before unix epoch: {error}"))?
        .as_secs();
    let record = QuarantineRecord::new(range.start, range.end, at_seq, detected_at_ts)
        .map_err(|error| error.to_string())?;
    ManifestStore::open(vault)
        .append_quarantine(record)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn print_verify_result(result: VerifyResult) -> crate::error::CliResult {
    match result {
        VerifyResult::Intact { count } => {
            println!("CHAIN_INTACT count={count}");
            Ok(())
        }
        VerifyResult::Broken { at_seq, .. } => Err(CalyxError::ledger_chain_broken(format!(
            "ledger chain broken at seq={at_seq}"
        ))
        .into()),
        VerifyResult::Corrupt { at_seq, reason } => Err(CalyxError::ledger_corrupt(format!(
            "ledger corrupt at seq={at_seq}: {reason}"
        ))
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use calyx_aster::ledger_view::parse_aster_ledger_seq;

    use super::*;

    #[test]
    fn parse_seq_accepts_u64() {
        assert_eq!(parse_seq("7").unwrap(), 7);
    }

    #[test]
    fn aster_ledger_keys_are_big_endian_u64() {
        assert_eq!(parse_aster_ledger_seq(&9_u64.to_be_bytes()).unwrap(), 9);
    }

    #[test]
    fn aster_ledger_keys_reject_wrong_width() {
        let error = parse_aster_ledger_seq(&[1, 2, 3]).unwrap_err();
        assert_eq!(error.code, "CALYX_LEDGER_CORRUPT");
        assert!(error.to_string().contains("expected 8"));
    }
}
