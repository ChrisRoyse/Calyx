use std::env;
use std::ops::Range;
use std::path::Path;

use calyx_ledger::{DirectoryLedgerStore, merkle_root};

use crate::cf_read::hex_bytes;
use crate::ledger_store::AsterLedgerCfStore;

pub fn print_root(ledger_dir: &Path, range: Range<u64>) -> crate::error::CliResult {
    let store = DirectoryLedgerStore::open(ledger_dir)?;
    let root = merkle_root(&store, range)?;
    println!("{}", hex_bytes(&root));
    Ok(())
}

pub fn print_root_from_env(range: Range<u64>) -> crate::error::CliResult {
    let ledger_dir = env::var("CALYX_LEDGER_DIR")
        .map_err(|_| "CALYX_LEDGER_DIR is required when --ledger is omitted".to_string())?;
    print_root(Path::new(&ledger_dir), range)
}

pub fn print_root_from_vault(vault: &Path, range: Range<u64>) -> crate::error::CliResult {
    let store = AsterLedgerCfStore::open(vault)?;
    let root = merkle_root(&store, range)?;
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
