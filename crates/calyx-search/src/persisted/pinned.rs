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
const DEFAULT_PIN_BUDGET_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const PIN_BUDGET_CODE: &str = "CALYX_SEARCH_PIN_BUDGET_EXCEEDED";
const PIN_BUDGET_REMEDIATION: &str = "raise CALYX_SEARCH_PIN_BUDGET_BYTES for this process or retire/re-segment the oversized lens; pinned verified search indexes must fit the configured budget";

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
pub(super) fn release(key: &PinKey) {
    ledger().lock().expect("pin ledger poisoned").remove(key);
}

/// Structured fail-closed error for a pinned-index allocation the OS refused.
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
        code: PIN_BUDGET_CODE,
        message: message.into(),
        remediation: PIN_BUDGET_REMEDIATION,
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
        assert_eq!(err.code(), PIN_BUDGET_CODE);
        assert!(err.message().contains("exceeding budget 1000 bytes"));
        let CliError::Calyx(calyx) = err else {
            panic!("expected structured Calyx error");
        };
        assert_eq!(calyx.remediation, PIN_BUDGET_REMEDIATION);
        assert_eq!(ledger.lock().unwrap().get(&second), None);
    }
}
