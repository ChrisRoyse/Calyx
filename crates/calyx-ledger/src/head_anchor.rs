use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::entry::HASH_BYTES;

/// External witness for the append-only ledger head.
///
/// The hash chain proves row modification and middle deletion, but the newest
/// rows can be removed unless a head outside the ledger row set records the
/// highest committed height and tip hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerHeadAnchor {
    pub height: u64,
    pub tip_hash: [u8; HASH_BYTES],
}

impl LedgerHeadAnchor {
    pub fn new(height: u64, tip_hash: [u8; HASH_BYTES]) -> Result<Self> {
        if height == 0 && tip_hash != [0_u8; HASH_BYTES] {
            return Err(CalyxError::ledger_corrupt(
                "ledger head anchor height 0 must use the genesis hash",
            ));
        }
        Ok(Self { height, tip_hash })
    }
}

pub fn verify_recovered_tip(
    anchor: Option<&LedgerHeadAnchor>,
    current_head: u64,
    current_tip_hash: [u8; HASH_BYTES],
) -> Result<()> {
    let Some(anchor) = anchor else {
        return Ok(());
    };
    if current_head < anchor.height {
        return Err(CalyxError::ledger_chain_broken(format!(
            "ledger end-truncated: current head {current_head} is below anchored head {}",
            anchor.height
        )));
    }
    if current_head == anchor.height && current_tip_hash != anchor.tip_hash {
        return Err(CalyxError::ledger_chain_broken(
            "ledger tip hash does not match anchored head",
        ));
    }
    Ok(())
}

pub(crate) fn read_anchor_file(path: &Path) -> Result<Option<LedgerHeadAnchor>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)
        .map_err(|error| CalyxError::disk_pressure(format!("read ledger head anchor: {error}")))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| CalyxError::ledger_corrupt(format!("decode ledger head anchor: {error}")))
}
