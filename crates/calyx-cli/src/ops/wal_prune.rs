//! Top-level `calyx wal-prune --vault <V>` catch-up command (issue #1992).
//!
//! Mirrors `calyx compact`'s shape for the WAL: opens the vault write-capable,
//! then drives bounded WAL-prune rounds until every segment below the verified
//! durable coverage floor is retired, printing one `PRUNED` line per round.
//! Idempotent — a vault with nothing to prune exits rc=0 having printed a
//! single no-op line.

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::{WalPruneConfig, WalPruneOutcome};

use crate::cmd::vault::{home_dir, resolve_vault_info, vault_salt};
use crate::error::CliResult;

/// Per-round segment cap for the catch-up command. A debt-laden vault
/// (hundreds of covered segments) retires over successive bounded rounds so
/// each round's directory fsync stays bounded and every round is observable.
const CATCH_UP_ROUND_MAX_SEGMENTS: usize = 256;
/// Per-round byte cap (16 GiB): high enough for a healthy vault to drain in one
/// round, bounded enough to keep a 27 GB catch-up run's rounds visible.
const CATCH_UP_ROUND_MAX_BYTES: u64 = 16 * 1024 * 1024 * 1024;

pub fn wal_prune(vault: &str) -> CliResult {
    let config = WalPruneConfig {
        max_segments: CATCH_UP_ROUND_MAX_SEGMENTS,
        max_bytes: CATCH_UP_ROUND_MAX_BYTES,
    };
    let handle = open_write_capable(vault)?;
    run_rounds(&handle, config, vault)
}

fn open_write_capable(vault: &str) -> CliResult<AsterVault> {
    let home = home_dir()?;
    let resolved = resolve_vault_info(&home, vault)?;
    // write_latest_readback keeps the open fast (no MVCC restore) while staying
    // write-capable so the flush inside prune can advance the manifest.
    Ok(AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::write_latest_readback(),
    )?)
}

fn run_rounds(vault: &AsterVault, config: WalPruneConfig, label: &str) -> CliResult {
    let mut total_segments = 0usize;
    let mut total_bytes = 0u64;
    let mut rounds = 0usize;
    loop {
        let outcome = vault.prune_covered_wal_segments(config)?;
        if outcome.segments_pruned == 0 {
            if rounds == 0 {
                print_noop(label, &outcome);
            }
            print_done(label, rounds, total_segments, total_bytes, &outcome);
            return Ok(());
        }
        rounds += 1;
        total_segments += outcome.segments_pruned;
        total_bytes = total_bytes.saturating_add(outcome.bytes_reclaimed);
        print_round(label, &outcome);
        // Coverage cannot advance without new writes, so once a round retires
        // the entire covered prefix the run is complete.
        if outcome.segments_pruned >= outcome.prunable_segments_before {
            print_done(label, rounds, total_segments, total_bytes, &outcome);
            return Ok(());
        }
    }
}

fn print_round(label: &str, outcome: &WalPruneOutcome) {
    println!(
        "PRUNED\tVAULT\t{label}\tSEGMENTS\t{}\tBYTES\t{}\tFLOOR_SEQ\t{}\tSEGMENTS_BEFORE\t{}\tSEGMENTS_AFTER\t{}",
        outcome.segments_pruned,
        outcome.bytes_reclaimed,
        outcome.coverage_floor_seq,
        outcome.segments_before,
        outcome.segments_after
    );
}

fn print_noop(label: &str, outcome: &WalPruneOutcome) {
    println!(
        "PRUNE_NOOP\tVAULT\t{label}\tFLOOR_SEQ\t{}\tSEGMENTS\t{}\tBYTES\t{}",
        outcome.coverage_floor_seq, outcome.segments_before, outcome.bytes_before
    );
}

fn print_done(
    label: &str,
    rounds: usize,
    total_segments: usize,
    total_bytes: u64,
    outcome: &WalPruneOutcome,
) {
    println!(
        "PRUNE_DONE\tVAULT\t{label}\tROUNDS\t{rounds}\tSEGMENTS_PRUNED\t{total_segments}\tBYTES_RECLAIMED\t{total_bytes}\tSEGMENTS_REMAINING\t{}\tBYTES_REMAINING\t{}",
        outcome.segments_after, outcome.bytes_after
    );
}
