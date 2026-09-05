//! Process-wide accounting for verified, pinned search-index data.
//!
//! Fail-closed contract: every byte pinned in memory was verified against the
//! manifest's sha256 at load time. Pins are keyed by (canonical vault dir,
//! slot, kind) and hold exactly one generation: loading a new entry sha for
//! the same key replaces the previous pin. Pinning never falls back — if the
//! configured budget would be exceeded the load fails with a structured
//! error instead of silently degrading to per-query disk scans.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use calyx_core::CalyxError;

use crate::error::{CliError, CliResult};

pub(super) const PIN_BUDGET_ENV: &str = "CALYX_SEARCH_PIN_BUDGET_BYTES";
// The CPU MaxSim serving path now streams each segment row-by-row and never
// pins the lens resident (issue #1845), so the old 32 GiB default — sized to
// hold one ~26.2 GB MaxSim generation resident — is obsolete. The budget now
// bounds only the genuinely retained pins (flat-dense and sparse sidecars) plus
// the small streaming working sets, so it is an explicit, enforceable ceiling
// rather than a rubber stamp. Operators serving an unusually large dense/sparse
// panel raise CALYX_SEARCH_PIN_BUDGET_BYTES; the failure is fail-closed, never a
// silent per-query disk scan and never a dropped lens.
const DEFAULT_PIN_BUDGET_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub const SEARCH_PIN_BUDGET_EXCEEDED_CODE: &str = "CALYX_SEARCH_PIN_BUDGET_EXCEEDED";
pub const SEARCH_PIN_BUDGET_EXCEEDED_REMEDIATION: &str = "raise CALYX_SEARCH_PIN_BUDGET_BYTES for this process or retire/re-segment the oversized lens; pinned verified search indexes must fit the configured budget";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct PinKey {
    vault_dir: String,
    slot: u16,
    kind: &'static str,
}

impl PinKey {
    pub(super) fn new(vault_dir: &Path, slot: u16, kind: &'static str) -> CliResult<Self> {
        Ok(Self {
            vault_dir: canonical_vault_dir(vault_dir)?,
            slot,
            kind,
        })
    }
}

type PinLedger = Mutex<BTreeMap<PinKey, u64>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PinBudgetPreflight {
    pub(crate) required_bytes: u64,
    pub(crate) projected_process_bytes: u64,
    pub(crate) configured_bytes: u64,
}

fn ledger() -> &'static PinLedger {
    static LEDGER: OnceLock<PinLedger> = OnceLock::new();
    LEDGER.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Reserve `bytes` for a pin at `key`, replacing any previous reservation for
/// the same key. Fails closed with `CALYX_SEARCH_PIN_BUDGET_EXCEEDED` when the
/// process-wide budget would be exceeded.
pub(super) fn reserve(key: &PinKey, bytes: u64) -> CliResult {
    reserve_in_ledger(ledger(), key, bytes, configured_pin_budget_bytes()?)
}

/// Fail before hashing or loading any sidecar when the selected immutable
/// generation cannot fit the configured process pin budget. Requirements
/// replace reservations for the same `(vault, slot, kind)` keys, matching the
/// later `reserve` contract exactly.
pub(super) fn preflight(
    vault_dir: &Path,
    requirements: &[(u16, &'static str, u64)],
) -> CliResult<PinBudgetPreflight> {
    let canonical = canonical_vault_dir(vault_dir)?;
    let configured_bytes = configured_pin_budget_bytes()?;
    let mut projected = ledger().lock().expect("pin ledger poisoned").clone();
    let mut required_bytes = 0u64;
    for &(slot, kind, bytes) in requirements {
        required_bytes = required_bytes.checked_add(bytes).ok_or_else(|| {
            pin_budget_error(format!(
                "search pin preflight byte accounting overflowed for selected slot {slot} kind {kind}"
            ))
        })?;
        projected.insert(
            PinKey {
                vault_dir: canonical.clone(),
                slot,
                kind,
            },
            bytes,
        );
    }
    let projected_process_bytes = projected.values().try_fold(0u64, |total, bytes| {
        total.checked_add(*bytes).ok_or_else(|| {
            pin_budget_error("search pin preflight projected process byte accounting overflowed")
        })
    })?;
    if projected_process_bytes > configured_bytes {
        return Err(pin_budget_error(format!(
            "search pin preflight for {} requires {required_bytes} bytes for the selected generation; projected process pinned total is {projected_process_bytes} bytes, exceeding configured budget {configured_bytes} bytes",
            canonical
        )));
    }
    Ok(PinBudgetPreflight {
        required_bytes,
        projected_process_bytes,
        configured_bytes,
    })
}

fn reserve_in_ledger(ledger: &PinLedger, key: &PinKey, bytes: u64, budget: u64) -> CliResult {
    let mut ledger = ledger.lock().expect("pin ledger poisoned");
    let others: u64 = ledger
        .iter()
        .filter(|(existing, _)| *existing != key)
        .map(|(_, bytes)| *bytes)
        .sum();
    let total = others.checked_add(bytes).ok_or_else(|| {
        pin_budget_error(format!(
            "pinned search index byte accounting overflowed adding {bytes} bytes for slot {} kind {}",
            key.slot, key.kind
        ))
    })?;
    if total > budget {
        return Err(pin_budget_error(format!(
            "pinning verified {} index for slot {} in {} needs {bytes} bytes; process pinned total would be {total} bytes, exceeding budget {budget} bytes",
            key.kind, key.slot, key.vault_dir
        )));
    }
    ledger.insert(key.clone(), bytes);
    Ok(())
}

/// Drop the reservation for `key` (used when a load fails after reserving).
/// Only the resident CUDA candidate-reuse path reserves-then-releases a multi
/// pin; the CPU serving path streams and never reserves (issue #1845).
#[cfg(feature = "cuda")]
pub(super) fn release(key: &PinKey) {
    ledger().lock().expect("pin ledger poisoned").remove(key);
}

/// Structured fail-closed error for a pinned-index allocation the OS refused.
#[cfg(feature = "cuda")]
pub(super) fn pin_allocation_error(key: &PinKey, bytes: u64) -> CliError {
    pin_budget_error(format!(
        "allocating {bytes} bytes for the verified {} index pin for slot {} in {} failed; the process is out of memory for the configured pin budget",
        key.kind, key.slot, key.vault_dir
    ))
}

fn configured_pin_budget_bytes() -> CliResult<u64> {
    let Some(raw) = std::env::var_os(PIN_BUDGET_ENV) else {
        return Ok(DEFAULT_PIN_BUDGET_BYTES);
    };
    let raw = raw.to_string_lossy().into_owned();
    raw.trim().parse::<u64>().map_err(|err| {
        pin_budget_error(format!(
            "{PIN_BUDGET_ENV}={raw} is not a valid byte count: {err}"
        ))
    })
}

pub(crate) fn canonical_vault_dir(vault_dir: &Path) -> CliResult<String> {
    let canonical = std::fs::canonicalize(vault_dir).map_err(|err| {
        pin_budget_error(format!(
            "pinned search index cannot canonicalize vault path {}: {err}",
            vault_dir.display()
        ))
    })?;
    Ok(canonical.display().to_string())
}

fn pin_budget_error(message: impl Into<String>) -> CliError {
    CalyxError {
        code: SEARCH_PIN_BUDGET_EXCEEDED_CODE,
        message: message.into(),
        remediation: SEARCH_PIN_BUDGET_EXCEEDED_REMEDIATION,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(kind: &'static str, slot: u16) -> PinKey {
        PinKey {
            vault_dir: "test-vault".to_string(),
            slot,
            kind,
        }
    }

    #[test]
    fn reserve_replaces_previous_generation_for_same_key() {
        let ledger = Mutex::new(BTreeMap::new());
        let pin = key("test_multi", 22);
        reserve_in_ledger(&ledger, &pin, 900, 1000).unwrap();
        reserve_in_ledger(&ledger, &pin, 950, 1000).unwrap();
        assert_eq!(ledger.lock().unwrap().get(&pin), Some(&950));
    }

    #[test]
    fn reserve_fails_closed_when_budget_exceeded() {
        let ledger = Mutex::new(BTreeMap::new());
        let first = key("test_multi", 22);
        let second = key("test_sparse", 13);
        reserve_in_ledger(&ledger, &first, 800, 1000).unwrap();
        let err = reserve_in_ledger(&ledger, &second, 300, 1000).unwrap_err();
        assert_eq!(err.code(), SEARCH_PIN_BUDGET_EXCEEDED_CODE);
        assert!(err.message().contains("exceeding budget 1000 bytes"));
        let CliError::Calyx(calyx) = err else {
            panic!("expected structured Calyx error");
        };
        assert_eq!(calyx.remediation, SEARCH_PIN_BUDGET_EXCEEDED_REMEDIATION);
        assert_eq!(ledger.lock().unwrap().get(&second), None);
    }
}
