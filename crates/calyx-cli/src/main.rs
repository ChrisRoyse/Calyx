//! Calyx command-line entry point.

mod anneal_ab_log;
mod anneal_autotune_report;
mod anneal_bandit_readback;
mod anneal_commands;
mod anneal_deficit_map;
mod anneal_frozen_guard_readback;
mod anneal_head_readback;
mod anneal_intelligence_report;
mod anneal_lens_proposal_log;
mod anneal_mistakes_readback;
mod anneal_propose_lens_fixture;
mod anneal_propose_lens_run;
mod anneal_propose_preview;
mod anneal_regression_readback;
mod anneal_replay_readback;
mod anneal_soak_report;
mod anneal_status;
mod budget_readback;
mod crash;
mod dedup_audit_readback;
mod dedup_readback;
mod fsv;
mod kernel_health_readback;
mod ledger_store;
mod manifest_readback;
mod merkle;
mod ops;
mod ph42_readback;
mod provenance;
mod recurrence_readback;
mod scan;
mod temporal_readback;
mod time_prediction_readback;
mod tripwire_readback;
mod usage;
mod vault_tree;
mod verify;
mod ward_tau_readback;

#[cfg(test)]
mod main_tests;

use std::env;
use std::fs;
use std::io;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.as_slice() {
        [command, flag, value] if command == "readback" && flag == "--hex" => {
            readback_hex(Path::new(value)).map_err(|error| error.to_string())
        }
        [command, flag, value] if command == "readback" && flag == "--vault-tree" => {
            vault_tree::readback_vault_tree(Path::new(value)).map_err(|error| error.to_string())
        }
        [command, topic, field_flag, field, vault_flag, vault]
            if command == "readback"
                && topic == "vault-manifest"
                && field_flag == "--field"
                && vault_flag == "--vault" =>
        {
            manifest_readback::readback_vault_manifest_field(Path::new(vault), field)
        }
        [command, topic, explain_flag, clock_flag, clock, tz_flag, tz]
            if command == "readback"
                && topic == "temporal_search"
                && explain_flag == "--explain"
                && clock_flag == "--clock-fixed"
                && tz_flag == "--tz-offset" =>
        {
            temporal_readback::readback_temporal_search(parse_i64(clock)?, parse_i32(tz)?)
        }
        [
            command,
            topic,
            vault_flag,
            vault,
            cx_flag,
            cx_id,
            slot_flag,
            slot,
            tau_flag,
            tau,
            near_flag,
            near_cos,
            distinct_flag,
            distinct_cos,
            vault_id_flag,
            vault_id,
            salt_flag,
            salt,
        ] if command == "readback"
            && topic == "dedup-check"
            && vault_flag == "--vault"
            && cx_flag == "--cx-id"
            && slot_flag == "--slot"
            && tau_flag == "--tau"
            && near_flag == "--near-cos"
            && distinct_flag == "--distinct-cos"
            && vault_id_flag == "--vault-id"
            && salt_flag == "--salt" =>
        {
            dedup_readback::readback_dedup_check(dedup_readback::DedupReadbackArgs {
                vault: Path::new(vault),
                cx_id,
                slot,
                tau,
                near_cos,
                distinct_cos,
                vault_id,
                salt,
            })
        }
        [command, topic, root_flag, root, kernel_flag, kernel_id]
            if command == "readback"
                && topic == "kernel-health"
                && root_flag == "--root"
                && kernel_flag == "--kernel-id" =>
        {
            kernel_health_readback::readback_kernel_health(Path::new(root), kernel_id)
        }
        [command, topic, vault_flag, vault, cx_flag, cx_id]
            if command == "readback"
                && topic == "recurrence-series"
                && vault_flag == "--vault"
                && cx_flag == "--cx-id" =>
        {
            recurrence_readback::readback_recurrence_series(Path::new(vault), cx_id)
        }
        [command, topic, rest @ ..] if command == "readback" && topic == "periodic-recall" => {
            recurrence_readback::readback_periodic_recall(rest)
        }
        [command, topic, rest @ ..] if command == "readback" && ph42_readback::is_topic(topic) => {
            ph42_readback::readback_topic(topic, rest)
        }
        [command, topic, name, vault_flag, vault]
            if command == "readback" && topic == "config" && vault_flag == "--vault" =>
        {
            readback_config(name, Path::new(vault))
        }
        [command, topic, rest @ ..] if command == "anneal" => anneal_commands::run(topic, rest),
        [command, topic, subtopic, vault_flag, vault, last_flag, last]
            if command == "readback"
                && topic == "anneal"
                && subtopic == "mistakes"
                && vault_flag == "--vault"
                && last_flag == "--last" =>
        {
            anneal_mistakes_readback::readback_mistakes(
                Path::new(vault),
                anneal_status::parse_last(last)?,
            )
        }
        [command, topic, slot_flag, slot, vault_flag, vault]
            if command == "ward"
                && topic == "tau"
                && slot_flag == "--slot"
                && vault_flag == "--vault" =>
        {
            ward_tau_readback::readback_ward_tau(Path::new(vault), slot)
        }
        [
            command,
            topic,
            vault_flag,
            vault,
            cx_flag,
            cx_id,
            ceiling_flag,
            ceiling,
        ] if command == "readback"
            && topic == "time-prediction"
            && vault_flag == "--vault"
            && cx_flag == "--cx-id"
            && ceiling_flag == "--confidence-ceiling" =>
        {
            time_prediction_readback::readback_time_prediction(Path::new(vault), cx_id, ceiling)
        }
        [command, topic, vault_flag, vault, cx_flag, cx_id]
            if command == "readback"
                && topic == "dedup-audit"
                && vault_flag == "--vault"
                && cx_flag == "--cx-id" =>
        {
            dedup_audit_readback::readback_dedup_audit(Path::new(vault), cx_id)
        }
        [command, topic, vault_flag, vault, token_flag, token]
            if command == "readback"
                && topic == "dedup-undo"
                && vault_flag == "--vault"
                && token_flag == "--token" =>
        {
            dedup_audit_readback::readback_dedup_undo(Path::new(vault), token)
        }
        [command, topic, vault_flag, vault]
            if command == "readback" && topic == "cx-list" && vault_flag == "--vault" =>
        {
            dedup_audit_readback::readback_cx_list(Path::new(vault))
        }
        [command, flag, cf, vault_flag, vault]
            if command == "readback" && flag == "--cf" && vault_flag == "--vault" =>
        {
            ops::readback_cf(Path::new(vault), cf)
        }
        [command, flag, cf, vault_flag, vault, seq_flag, seq]
            if command == "readback"
                && flag == "--cf"
                && cf == "ledger"
                && vault_flag == "--vault"
                && seq_flag == "--seq" =>
        {
            verify::readback_ledger_seq(Path::new(vault), verify::parse_seq(seq)?)
        }
        [command, flag, vault_flag, vault]
            if command == "readback" && flag == "--wal" && vault_flag == "--vault" =>
        {
            ops::readback_wal(Path::new(vault))
        }
        [command, flag, cf, level_flag, level_dir]
            if command == "readback" && flag == "--cf" && level_flag == "--level" =>
        {
            fsv::readback_level(cf, Path::new(level_dir))
        }
        [command, ledger_flag, ledger, range_flag, range]
            if command == "merkle-root" && ledger_flag == "--ledger" && range_flag == "--range" =>
        {
            merkle::print_root(Path::new(ledger), merkle::parse_range(range)?)
        }
        [command, vault_flag, vault, range_flag, range]
            if command == "merkle-root" && vault_flag == "--vault" && range_flag == "--range" =>
        {
            merkle::print_root_from_vault(Path::new(vault), merkle::parse_range(range)?)
        }
        [command, range_flag, range] if command == "merkle-root" && range_flag == "--range" => {
            merkle::print_root_from_env(merkle::parse_range(range)?)
        }
        [command, ledger_flag, ledger, range_flag, range]
            if command == "verify-chain"
                && ledger_flag == "--ledger"
                && range_flag == "--range" =>
        {
            verify::verify_ledger_dir(Path::new(ledger), verify::parse_verify_range(range)?)
        }
        [command, vault_flag, vault, range_flag, range]
            if command == "verify-chain" && vault_flag == "--vault" && range_flag == "--range" =>
        {
            verify::verify_vault(Path::new(vault), verify::parse_verify_range(range)?)
        }
        [command, cf_flag, cf, vault_flag, vault]
            if command == "scan"
                && cf_flag == "--cf"
                && cf == "ledger"
                && vault_flag == "--vault" =>
        {
            scan::scan_ledger_vault(Path::new(vault))
        }
        [command, vault_flag, vault, cx_flag, cx]
            if command == "get-provenance" && vault_flag == "--vault" && cx_flag == "--cx" =>
        {
            provenance::get_provenance(Path::new(vault), cx)
        }
        [command, vault_flag, vault, answer_flag, answer]
            if command == "get-answer-trace"
                && vault_flag == "--vault"
                && answer_flag == "--answer" =>
        {
            provenance::get_answer_trace(Path::new(vault), answer)
        }
        [command, vault_flag, vault, kind_flag, kind]
            if command == "audit" && vault_flag == "--vault" && kind_flag == "--kind" =>
        {
            provenance::audit(Path::new(vault), kind)
        }
        [command, vault_flag, vault, cf_flag, cf]
            if command == "compact" && vault_flag == "--vault" && cf_flag == "--cf" =>
        {
            ops::compact(Path::new(vault), cf)
        }
        [command, vault_flag, vault, duration_flag, duration]
            if command == "compact-watch"
                && vault_flag == "--vault"
                && duration_flag == "--duration" =>
        {
            ops::compact_watch(Path::new(vault), ops::parse_duration(duration)?)
        }
        [
            command,
            vault_flag,
            vault,
            ops_flag,
            ops,
            threads_flag,
            threads,
        ] if command == "soak"
            && vault_flag == "--vault"
            && ops_flag == "--ops"
            && threads_flag == "--threads" =>
        {
            let ops = ops
                .parse::<usize>()
                .map_err(|error| format!("invalid --ops: {error}"))?;
            let threads = threads
                .parse::<usize>()
                .map_err(|error| format!("invalid --threads: {error}"))?;
            ops::soak(Path::new(vault), ops, threads)
        }
        [command, vault_flag, vault, cf_flag, cf, output_flag, output]
            if command == "tier"
                && vault_flag == "--vault"
                && cf_flag == "--cf"
                && output_flag == "--output" =>
        {
            ops::tier(Path::new(vault), cf, output)
        }
        [command, vault_flag, vault] if command == "vault-demo" && vault_flag == "--vault" => {
            ops::vault_demo(Path::new(vault))
        }
        [command, vault_flag, vault] if command == "arrow-demo" && vault_flag == "--vault" => {
            fsv::arrow_demo(Path::new(vault))
        }
        [command, vault_flag, vault] if command == "cf-demo" && vault_flag == "--vault" => {
            fsv::cf_demo(Path::new(vault))
        }
        [command, vault_flag, vault] if command == "mvcc-demo" && vault_flag == "--vault" => {
            fsv::mvcc_demo(Path::new(vault))
        }
        [command, vault_flag, vault, records_flag, records]
            if command == "wal-drill" && vault_flag == "--vault" && records_flag == "--records" =>
        {
            let records = records
                .parse::<usize>()
                .map_err(|error| format!("invalid --records: {error}"))?;
            fsv::wal_drill(Path::new(vault), records)
        }
        [command, wal_dir] if command == "wal-replay" => fsv::wal_replay(Path::new(wal_dir)),
        [
            command,
            vault_flag,
            vault,
            point_flag,
            point,
            pause_flag,
            pause_ms,
        ] if command == "crash-drill"
            && vault_flag == "--vault"
            && point_flag == "--point"
            && pause_flag == "--pause-ms" =>
        {
            let pause_ms = pause_ms
                .parse::<u64>()
                .map_err(|error| format!("invalid --pause-ms: {error}"))?;
            crash::crash_drill(
                Path::new(vault),
                crash::CrashPoint::parse(point)?,
                Some(pause_ms),
            )
        }
        [command, vault_flag, vault, point_flag, point]
            if command == "crash-drill" && vault_flag == "--vault" && point_flag == "--point" =>
        {
            crash::crash_drill(Path::new(vault), crash::CrashPoint::parse(point)?, None)
        }
        [command, vault_flag, vault] if command == "recover" && vault_flag == "--vault" => {
            crash::recover(Path::new(vault))
        }
        [command, vault_flag, vault, index_flag, index]
            if command == "open-check" && vault_flag == "--vault" && index_flag == "--index" =>
        {
            let index = index
                .parse::<u8>()
                .map_err(|error| format!("invalid --index: {error}"))?;
            crash::open_check(Path::new(vault), index)
        }
        [command, vault_flag, vault, cf_flag, cf, offset_flag, offset]
            if command == "corrupt-shard"
                && vault_flag == "--vault"
                && cf_flag == "--cf"
                && offset_flag == "--byte-offset" =>
        {
            let offset = offset
                .parse::<u64>()
                .map_err(|error| format!("invalid --byte-offset: {error}"))?;
            fsv::corrupt_shard(Path::new(vault), cf, offset)
        }
        [command, vault_flag, vault, requests_flag, requests]
            if command == "wal-batch-demo"
                && vault_flag == "--vault"
                && requests_flag == "--requests" =>
        {
            let requests = requests
                .parse::<usize>()
                .map_err(|error| format!("invalid --requests: {error}"))?;
            ops::wal_batch_demo(Path::new(vault), requests)
        }
        [] | [_]
            if args
                .first()
                .is_none_or(|arg| arg == "--help" || arg == "-h") =>
        {
            usage::print_usage();
            Ok(())
        }
        _ => Err(usage::usage().to_string()),
    }
}

fn readback_hex(path: &Path) -> io::Result<()> {
    let bytes = fs::read(path)?;
    for line in hex_lines(&bytes) {
        println!("{line}");
    }
    Ok(())
}

fn parse_i64(value: &str) -> Result<i64, String> {
    value
        .parse::<i64>()
        .map_err(|error| format!("invalid i64 value {value}: {error}"))
}

fn parse_i32(value: &str) -> Result<i32, String> {
    value
        .parse::<i32>()
        .map_err(|error| format!("invalid i32 value {value}: {error}"))
}

fn readback_config(name: &str, vault: &Path) -> Result<(), String> {
    match name {
        "tripwire" => tripwire_readback::readback_tripwire_config(vault),
        "budget" => budget_readback::readback_budget_config(vault),
        _ => Err(format!("unknown config readback: {name}")),
    }
}

fn hex_lines(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(32)
        .map(|chunk| {
            let mut line = String::with_capacity(chunk.len() * 2);
            for byte in chunk {
                line.push(hex_digit(byte >> 4));
                line.push(hex_digit(byte & 0x0f));
            }
            line
        })
        .collect()
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
