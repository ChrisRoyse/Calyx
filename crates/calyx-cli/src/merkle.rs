use std::env;
use std::ops::Range;
use std::path::Path;

use calyx_ledger::{DirectoryLedgerStore, merkle_root};

use crate::ledger_store::AsterLedgerCfStore;

pub fn print_root(ledger_dir: &Path, range: Range<u64>) -> Result<(), String> {
    let store = DirectoryLedgerStore::open(ledger_dir).map_err(|error| error.to_string())?;
    let root = merkle_root(&store, range).map_err(|error| error.to_string())?;
    println!("{}", hex_bytes(&root));
    Ok(())
}

pub fn print_root_from_env(range: Range<u64>) -> Result<(), String> {
    let ledger_dir = env::var("CALYX_LEDGER_DIR")
        .map_err(|_| "CALYX_LEDGER_DIR is required when --ledger is omitted".to_string())?;
    print_root(Path::new(&ledger_dir), range)
}

pub fn print_root_from_vault(vault: &Path, range: Range<u64>) -> Result<(), String> {
    let store = AsterLedgerCfStore::open(vault).map_err(|error| error.to_string())?;
    let root = merkle_root(&store, range).map_err(|error| error.to_string())?;
    println!("{}", hex_bytes(&root));
    Ok(())
}

pub fn parse_range(value: &str) -> Result<Range<u64>, String> {
    let (start, end) = value
        .split_once("..")
        .ok_or_else(|| "range must use a..b syntax".to_string())?;
    let start = start
        .parse::<u64>()
        .map_err(|error| format!("invalid range start: {error}"))?;
    let end = end
        .parse::<u64>()
        .map_err(|error| format!("invalid range end: {error}"))?;
    if start > end {
        return Err(format!("range start {start} > end {end}"));
    }
    Ok(start..end)
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
    use super::*;

    #[test]
    fn parse_range_accepts_half_open_range() {
        assert_eq!(parse_range("0..4").unwrap(), 0..4);
    }

    #[test]
    fn parse_range_rejects_reverse_range() {
        let error = parse_range("5..4").unwrap_err();
        assert!(error.contains("start 5 > end 4"));
    }
}
