//! Periodic Ledger chain-verify cycle feeding the chain-verify gauge family.
//!
//! Every cycle re-opens each target's ledger store from disk (no cached
//! state — the bytes are the verdict), runs `calyx_ledger::verify_chain`
//! over the full `0..head` range, and records the outcome. Non-intact
//! outcomes are logged with their exact `CALYX_*` code so the journal names
//! what failed; the gauge goes to 0 in the same cycle (fail-closed).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_ledger::{DirectoryLedgerStore, LedgerCfStore, VerifyResult, verify_chain};

use calyxd::error::DaemonError;
use calyxd::metrics::{ChainVerifyMetrics, VerifyOutcome};

/// How a verify target's ledger rows are stored on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// Aster vault directory (`cf/ledger` SSTs + WAL).
    Vault,
    /// Standalone directory ledger (one `.ledger` file per seq).
    LedgerDir,
}

/// One ledger whose chain is verified every cycle.
#[derive(Debug, Clone)]
pub struct VerifyTarget {
    pub kind: TargetKind,
    pub path: PathBuf,
}

impl VerifyTarget {
    /// Label value identifying this target on every metric series.
    pub fn label(&self) -> String {
        self.path.display().to_string()
    }

    /// Startup validation: the target must already be a directory.
    /// Misconfiguration is a hard exit, not a 0-gauge (PH65 fail-closed).
    pub fn validate(&self) -> Result<(), DaemonError> {
        if !self.path.is_dir() {
            return Err(DaemonError::config_invalid(format!(
                "verify target {} is not a directory",
                self.path.display()
            )));
        }
        Ok(())
    }

    /// Opens the store fresh from disk and verifies the full chain.
    fn verify(&self) -> VerifyOutcome {
        let result = match self.kind {
            TargetKind::Vault => {
                AsterLedgerCfStore::open(&self.path).and_then(|store| verify_full_chain(&store))
            }
            TargetKind::LedgerDir => {
                DirectoryLedgerStore::open(&self.path).and_then(|store| verify_full_chain(&store))
            }
        };
        match result {
            Ok(VerifyResult::Intact { count }) => VerifyOutcome::Intact { entries: count },
            Ok(VerifyResult::Broken { at_seq, .. }) => VerifyOutcome::Broken { at_seq },
            Ok(VerifyResult::Corrupt { at_seq, reason }) => {
                VerifyOutcome::Corrupt { at_seq, reason }
            }
            Err(error) => VerifyOutcome::Error {
                detail: error.to_string(),
            },
        }
    }
}

/// Verifies `0..head` where head is one past the highest stored seq.
fn verify_full_chain(store: &dyn LedgerCfStore) -> calyx_core::Result<VerifyResult> {
    let head = store
        .scan()?
        .iter()
        .map(|row| row.seq)
        .max()
        .map_or(0, |max_seq| max_seq.saturating_add(1));
    verify_chain(store, 0..head)
}

/// Runs one verify cycle over all targets, recording each outcome.
pub fn run_cycle(targets: &[VerifyTarget], metrics: &ChainVerifyMetrics) {
    for target in targets {
        let outcome = target.verify();
        let now_secs = unix_now_secs();
        metrics.record(&target.label(), &outcome, now_secs);
        log_outcome(target, &outcome);
    }
}

/// Spawns the periodic verify loop on its own thread.
pub fn spawn_loop(
    targets: Vec<VerifyTarget>,
    metrics: Arc<ChainVerifyMetrics>,
    interval: Duration,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(interval);
            run_cycle(&targets, &metrics);
        }
    })
}

fn log_outcome(target: &VerifyTarget, outcome: &VerifyOutcome) {
    let label = target.label();
    match outcome {
        VerifyOutcome::Intact { entries } => {
            println!("calyxd: chain_verify vault={label} outcome=intact entries={entries}");
        }
        VerifyOutcome::Broken { at_seq } => {
            eprintln!(
                "calyxd: chain_verify vault={label} outcome=broken \
                 CALYX_LEDGER_CHAIN_BROKEN at seq={at_seq} — quarantine range, investigate"
            );
        }
        VerifyOutcome::Corrupt { at_seq, reason } => {
            eprintln!(
                "calyxd: chain_verify vault={label} outcome=corrupt \
                 CALYX_LEDGER_CORRUPT at seq={at_seq}: {reason}"
            );
        }
        VerifyOutcome::Error { detail } => {
            eprintln!("calyxd: chain_verify vault={label} outcome=error {detail}");
        }
    }
}

fn unix_now_secs() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX),
        Err(error) => {
            eprintln!("calyxd: system clock before unix epoch: {error}");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_missing_directory() {
        let target = VerifyTarget {
            kind: TargetKind::LedgerDir,
            path: PathBuf::from("Z:/does/not/exist-calyxd-602"),
        };
        let error = target.validate().unwrap_err();
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
    }

    #[test]
    fn vanished_target_records_error_outcome_not_panic() {
        let target = VerifyTarget {
            kind: TargetKind::Vault,
            path: PathBuf::from("Z:/vanished/vault-calyxd-602"),
        };
        let metrics = ChainVerifyMetrics::new(&[target.label()]);
        run_cycle(std::slice::from_ref(&target), &metrics);
        assert_eq!(metrics.ok_value_for(&target.label()), 0);
        assert_eq!(metrics.runs_for(&target.label(), "error"), 1);
    }
}
