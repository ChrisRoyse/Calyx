use std::ops::Range;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::manifest::{ManifestStore, QuarantineRecord, is_vault_seq_quarantined};
use calyx_core::CalyxError;
use calyx_ledger::{DirectoryLedgerStore, LedgerCfStore, VerifyResult, verify_chain};

use crate::ledger_store::AsterLedgerCfStore;
use crate::merkle::parse_range;

pub fn verify_ledger_dir(ledger: &Path, range: Range<u64>) -> Result<(), String> {
    let store = DirectoryLedgerStore::open(ledger).map_err(|error| error.to_string())?;
    print_verify_result(verify_chain(&store, range).map_err(|error| error.to_string())?)
}

pub fn verify_vault(vault: &Path, range: Range<u64>) -> Result<(), String> {
    let store = AsterLedgerCfStore::open(vault).map_err(|error| error.to_string())?;
    let result = verify_chain(&store, range.clone()).map_err(|error| error.to_string())?;
    if let Some(at_seq) = result.quarantine_seq() {
        write_quarantine(vault, range, at_seq)?;
    }
    print_verify_result(result)
}

pub fn readback_ledger_seq(vault: &Path, seq: u64) -> Result<(), String> {
    if is_vault_seq_quarantined(vault, seq).map_err(|error| error.to_string())? {
        return Err(
            CalyxError::ledger_chain_broken(format!("ledger seq {seq} is quarantined")).to_string(),
        );
    }
    let store = AsterLedgerCfStore::open(vault).map_err(|error| error.to_string())?;
    let rows = store.scan().map_err(|error| error.to_string())?;
    let row = rows
        .into_iter()
        .find(|row| row.seq == seq)
        .ok_or_else(|| format!("CALYX_LEDGER_CORRUPT: missing ledger row for seq {seq}"))?;
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

fn write_quarantine(vault: &Path, range: Range<u64>, at_seq: u64) -> Result<(), String> {
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

fn print_verify_result(result: VerifyResult) -> Result<(), String> {
    match result {
        VerifyResult::Intact { count } => {
            println!("CHAIN_INTACT count={count}");
            Ok(())
        }
        VerifyResult::Broken { at_seq, .. } => {
            Err(format!("CALYX_LEDGER_CHAIN_BROKEN at seq={at_seq}"))
        }
        VerifyResult::Corrupt { at_seq, reason } => {
            Err(format!("CALYX_LEDGER_CORRUPT at seq={at_seq}: {reason}"))
        }
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
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
