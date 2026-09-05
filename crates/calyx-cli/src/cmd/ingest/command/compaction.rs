//! Ingest-lifecycle flush-SST compaction (issue #1985).
//!
//! `calyx ingest` batch mode historically never compacted, so a large build
//! accumulated millions of uncompacted per-commit flush SSTs and every read
//! path degraded toward O(files). Batch ingest now measures per-CF flush-SST
//! debt at chunk-commit boundaries and drives bounded compaction rounds (under
//! the durable commit lock the vault already takes) whenever debt crosses a
//! trigger, plus a final barrier before the summary so ingest never exits
//! leaving a CF above the trigger.

use calyx_aster::vault::{AsterVault, CfCompactionOutcome, IngestCompactionConfig};

use super::ingest_runtime_log;
use crate::cmd::vault::ResolvedVault;
use crate::error::{CliError, CliResult};

/// Env override for the debt trigger (below-target SSTs per CF).
const TRIGGER_FILES_ENV: &str = "CALYX_INGEST_COMPACTION_TRIGGER_FILES";
/// Env override for the per-round input-file cap.
const ROUND_MAX_FILES_ENV: &str = "CALYX_INGEST_COMPACTION_ROUND_MAX_FILES";
/// Env override for the per-round input-byte cap.
const ROUND_MAX_BYTES_ENV: &str = "CALYX_INGEST_COMPACTION_ROUND_MAX_BYTES";

/// Default debt trigger. RocksDB triggers L0 compaction at 4 files; a batch
/// ingest commits many CFs per chunk, so a slightly higher default keeps the
/// per-chunk compaction cost down while staying far below the thousands that
/// degrade reads (issue #1985).
const DEFAULT_TRIGGER_FILES: usize = 12;
/// Default per-round input-file cap, so a CF with hundreds of thousands of
/// files compacts over successive rounds without opening every reader at once.
const DEFAULT_ROUND_MAX_FILES: usize = 4_096;
/// Default per-round input-byte cap (8 GiB).
const DEFAULT_ROUND_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Resolves the ingest compaction config from the environment, failing closed
/// (config error) on any set-but-unparseable or zero value (issue #1985 §1).
pub(super) fn resolve_ingest_compaction_config() -> CliResult<IngestCompactionConfig> {
    resolve_ingest_compaction_config_from(
        std::env::var(TRIGGER_FILES_ENV).ok(),
        std::env::var(ROUND_MAX_FILES_ENV).ok(),
        std::env::var(ROUND_MAX_BYTES_ENV).ok(),
    )
}

/// Pure resolver over already-read override values, so tests can exercise the
/// fail-closed parsing without mutating process-global environment (which
/// races every parallel test that runs an ingest).
pub(in crate::cmd::ingest) fn resolve_ingest_compaction_config_from(
    trigger_files: Option<String>,
    round_max_files: Option<String>,
    round_max_bytes: Option<String>,
) -> CliResult<IngestCompactionConfig> {
    let config = IngestCompactionConfig {
        trigger_files: parse_positive_usize(
            TRIGGER_FILES_ENV,
            trigger_files,
            DEFAULT_TRIGGER_FILES,
        )?,
        max_round_files: parse_positive_usize(
            ROUND_MAX_FILES_ENV,
            round_max_files,
            DEFAULT_ROUND_MAX_FILES,
        )?,
        max_round_bytes: parse_positive_u64(
            ROUND_MAX_BYTES_ENV,
            round_max_bytes,
            DEFAULT_ROUND_MAX_BYTES,
        )?,
    };
    // Fail closed on nonsensical combinations before ingest starts.
    config.validate().map_err(CliError::from)?;
    Ok(config)
}

fn parse_positive_usize(name: &str, raw: Option<String>, default: usize) -> CliResult<usize> {
    match raw {
        None => Ok(default),
        Some(raw) => raw
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|&value| value >= 1)
            .ok_or_else(|| invalid_env(name, &raw)),
    }
}

fn parse_positive_u64(name: &str, raw: Option<String>, default: u64) -> CliResult<u64> {
    match raw {
        None => Ok(default),
        Some(raw) => raw
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|&value| value >= 1)
            .ok_or_else(|| invalid_env(name, &raw)),
    }
}

fn invalid_env(name: &str, raw: &str) -> CliError {
    CliError::usage(format!(
        "{name} must be a positive integer, got {raw:?}; unset it for the default or provide a value >= 1"
    ))
}

/// Runs one ingest-lifecycle compaction pass over the vault and logs a
/// per-CF line for every CF that was touched or was already above the
/// trigger. `phase` distinguishes chunk-boundary passes from the final
/// barrier. Any compaction failure propagates (no warn-and-continue).
pub(super) fn run_ingest_compaction(
    resolved: &ResolvedVault,
    vault: &AsterVault,
    config: IngestCompactionConfig,
    phase: &str,
) -> CliResult<()> {
    let outcomes = vault.compact_ingest_flush_debt(config)?;
    for outcome in &outcomes {
        log_outcome(resolved, phase, outcome);
    }
    Ok(())
}

fn log_outcome(resolved: &ResolvedVault, phase: &str, outcome: &CfCompactionOutcome) {
    // Skip CFs that were already under the trigger and untouched, to keep the
    // per-chunk log readable; the final barrier still reports movers.
    if outcome.rounds == 0 && outcome.files_before == outcome.files_after {
        return;
    }
    ingest_runtime_log(format_args!(
        "phase={phase} vault={} cf={} files_before={} files_after={} debt_before={} debt_after={} rounds={} reclaimed_input_files={} elapsed_ms={}",
        resolved.path.display(),
        outcome.cf.name(),
        outcome.files_before,
        outcome.files_after,
        outcome.debt_before,
        outcome.debt_after,
        outcome.rounds,
        outcome.reclaimed_input_files,
        outcome.elapsed_ms
    ));
}
