use calyx_core::{CalyxError, Result};
use calyx_ledger::{LedgerCfStore, LedgerRow};

use super::core;
use crate::server::ToolError;

#[test]
fn default_range_propagates_chain_end_scan_failure() {
    let error = core::verify_chain_for_store(&UnscannableLedger, None, None)
        .expect_err("default verify range must fail when the ledger cannot be scanned");

    let ToolError::Calyx(error) = error else {
        panic!("scan failure must retain its typed Calyx error");
    };
    assert_eq!(error.code, "CALYX_LEDGER_CORRUPT");
    assert!(error.message.contains("injected chain-end scan failure"));
}

struct UnscannableLedger;

impl LedgerCfStore for UnscannableLedger {
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        Err(CalyxError::ledger_corrupt(
            "injected chain-end scan failure",
        ))
    }

    fn put_new(&mut self, _seq: u64, _bytes: &[u8]) -> Result<()> {
        unreachable!("verify_chain does not append")
    }
}
