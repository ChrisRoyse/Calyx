//! Ingest-lifecycle WAL segment pruning (issue #1992).
//!
//! `calyx ingest` advanced the durable coverage floor (checkpoint + #1985
//! flush-SST compaction) but never deleted the WAL segments that floor already
//! covered, so a large build left the whole write history on disk (tens of GB,
//! hundreds of segments back to index 0) and every reopen header-scanned it.
//! Batch ingest now retires covered WAL segments at the same chunk-commit
//! boundaries and final barrier where #1985 compaction runs, in bounded rounds
//! under the durable commit lock, with a pure fail-closed env resolver.
//!
//! A dedicated config (not folded into [`IngestCompactionConfig`]) is the right
//! shape: WAL pruning has no debt "trigger" and its bounds are segment- and
//! byte-count caps over WAL files, semantically disjoint from the SST-file
//! trigger/round knobs the compactor tunes.

use calyx_aster::vault::AsterVault;
use calyx_aster::wal::{WalPruneConfig, WalPruneOutcome};

use super::ingest_runtime_log;
use crate::cmd::vault::ResolvedVault;
use crate::error::{CliError, CliResult};

/// Env override for the per-round segment cap.
const ROUND_MAX_SEGMENTS_ENV: &str = "CALYX_INGEST_WAL_PRUNE_ROUND_MAX_SEGMENTS";
/// Env override for the per-round byte cap.
const ROUND_MAX_BYTES_ENV: &str = "CALYX_INGEST_WAL_PRUNE_ROUND_MAX_BYTES";

/// Default per-round segment cap, so a debt-laden vault (hundreds of covered
/// segments) retires over successive bounded rounds rather than unlinking
/// everything under one directory fsync.
const DEFAULT_ROUND_MAX_SEGMENTS: usize = 256;
/// Default per-round byte cap (16 GiB): high enough that a healthy vault drains
/// in one round, bounded enough to keep a catch-up run's rounds observable.
const DEFAULT_ROUND_MAX_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// Resolves the ingest WAL-prune config from the environment, failing closed on
/// any set-but-unparseable or zero value.
pub(super) fn resolve_ingest_wal_prune_config() -> CliResult<WalPruneConfig> {
    resolve_ingest_wal_prune_config_from(
        std::env::var(ROUND_MAX_SEGMENTS_ENV).ok(),
        std::env::var(ROUND_MAX_BYTES_ENV).ok(),
    )
}

/// Pure resolver over already-read override values, so tests exercise the
/// fail-closed parsing without mutating process-global environment (the #1985
/// a1b9d6e0 lesson: mutating env races every parallel test running an ingest).
pub(in crate::cmd::ingest) fn resolve_ingest_wal_prune_config_from(
    round_max_segments: Option<String>,
    round_max_bytes: Option<String>,
) -> CliResult<WalPruneConfig> {
    let config = WalPruneConfig {
        max_segments: parse_positive_usize(
            ROUND_MAX_SEGMENTS_ENV,
            round_max_segments,
            DEFAULT_ROUND_MAX_SEGMENTS,
        )?,
        max_bytes: parse_positive_u64(
            ROUND_MAX_BYTES_ENV,
            round_max_bytes,
            DEFAULT_ROUND_MAX_BYTES,
        )?,
    };
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

/// Drives WAL-prune rounds until nothing covered remains to retire, logging one
/// structured line per round that reclaimed segments. `phase` distinguishes
/// chunk-boundary passes from the final barrier. Any prune failure propagates
/// (no warn-and-continue); coverage that cannot be proven fails closed inside
/// the engine primitive.
pub(super) fn run_ingest_wal_prune(
    resolved: &ResolvedVault,
    vault: &AsterVault,
    config: WalPruneConfig,
    phase: &str,
) -> CliResult<()> {
    loop {
        let started = std::time::Instant::now();
        let outcome = vault.prune_covered_wal_segments(config)?;
        if outcome.segments_pruned == 0 {
            return Ok(());
        }
        log_outcome(resolved, phase, &outcome, started.elapsed().as_millis());
        // A bounded round that fully retired the covered prefix leaves nothing
        // more to do until coverage advances again; stop without another pass.
        if outcome.segments_pruned >= outcome.prunable_segments_before {
            return Ok(());
        }
    }
}

fn log_outcome(resolved: &ResolvedVault, phase: &str, outcome: &WalPruneOutcome, elapsed_ms: u128) {
    ingest_runtime_log(format_args!(
        "phase={phase} vault={} floor_seq={} segments_before={} segments_after={} bytes_reclaimed={} elapsed_ms={}",
        resolved.path.display(),
        outcome.coverage_floor_seq,
        outcome.segments_before,
        outcome.segments_after,
        outcome.bytes_reclaimed,
        elapsed_ms
    ));
}
