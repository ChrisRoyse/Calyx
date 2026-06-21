//! Argument dispatch for the `calyx` binary.

mod early;

use std::path::Path;

use crate::cli_support::readback_config;
use crate::error::{CliError, CliResult};
use crate::{
    anneal_commands, anneal_ledger_readback, anneal_mistakes_readback, anneal_status,
    assay_bits_validation, assay_corpus_build, assay_ensemble_card, assay_fbin_export,
    assay_gdelt_rows, assay_i8bin_ensemble_card, assay_multi_anchor_card, assay_stream_fbin, crash,
    dedup_audit_readback, fsv, fsv_corpus, healthcheck, htap_validation, intelligence_commands,
    leapable, lens_commands, lodestar_commands, media_commands, merkle, migrate, navigate, ops,
    oracle_readback, oracle_sufficiency_validation, panel_commands, partitioned_bench,
    ph42_readback, provenance, resource_drill, resource_status, scan, sextant_bench,
    sextant_commands, summarize_command, temporal_log_recurrence_readback,
    time_prediction_readback, timetravel_readback, trigger_readback, usage, verify,
    ward_guard_validation, ward_tau_readback,
};

pub(crate) fn run(args: Vec<String>) -> CliResult {
    if let Some(result) = early::try_run(args.as_slice()) {
        return result;
    }

    match args.as_slice() {
        [command, rest @ ..] if command == "healthcheck" => healthcheck::run(rest),
        [command, topic, rest @ ..] if command == "migrate" => migrate::run(topic, rest),
        [command, topic, vault_flag, vault]
            if command == "intelligence" && topic == "abundance" && vault_flag == "--vault" =>
        {
            intelligence_commands::abundance(Path::new(vault))
        }
        [command, topic, rest @ ..]
            if command == "readback" && oracle_readback::is_topic(topic) =>
        {
            oracle_readback::readback_oracle(topic, rest)
        }
        [command, topic, rest @ ..]
            if command == "readback" && topic == "temporal-log-recurrence" =>
        {
            temporal_log_recurrence_readback::readback_temporal_log_recurrence(rest)
        }
        [command, topic, rest @ ..] if command == "readback" && ph42_readback::is_topic(topic) => {
            ph42_readback::readback_topic(topic, rest)
        }
        [command, topic, rest @ ..] if command == "leapable" => leapable::run(topic, rest),
        [command, mode, rest @ ..] if command == "navigate" => navigate::run(mode, rest),
        [command, rest @ ..] if command == "build-bench-vault" => sextant_bench::run_build(rest),
        [command, rest @ ..] if command == "build-partitioned-vault" => {
            partitioned_bench::run_build(rest)
        }
        [command, topic, rest @ ..] if command == "bench" && topic == "partitioned-search" => {
            partitioned_bench::run_search(rest)
        }
        [command, topic, rest @ ..] if command == "bench" && topic == "partitioned-rrf" => {
            partitioned_bench::run_rrf(rest)
        }
        [command, topic, rest @ ..]
            if command == "bench" && topic == "partitioned-rrf-slot-truth" =>
        {
            partitioned_bench::run_rrf_slot_truth(rest)
        }
        [command, topic, rest @ ..] if command == "bench" => sextant_bench::run_bench(topic, rest),
        [command, topic, rest @ ..] if command == "sextant" => sextant_commands::run(topic, rest),
        [command, topic, rest @ ..] if command == "media" => media_commands::run(topic, rest),
        [command, topic, rest @ ..] if command == "fsv" && topic == "corpus-readback" => {
            fsv_corpus::run(rest)
        }
        [command, topic, rest @ ..] if command == "lodestar" => lodestar_commands::run(topic, rest),
        [command, topic, rest @ ..] if command == "assay" && topic == "bits-validate" => {
            assay_bits_validation::run(rest)
        }
        [command, topic, rest @ ..] if command == "assay" && topic == "ensemble-card" => {
            assay_ensemble_card::run(rest)
        }
        [command, topic, rest @ ..] if command == "assay" && topic == "i8bin-ensemble-card" => {
            assay_i8bin_ensemble_card::run(rest)
        }
        [command, topic, rest @ ..]
            if command == "assay" && topic == "multi-anchor-ensemble-card" =>
        {
            assay_multi_anchor_card::run(rest)
        }
        [command, topic, rest @ ..] if command == "assay" && topic == "corpus-build" => {
            assay_corpus_build::run(rest)
        }
        [command, topic, rest @ ..] if command == "assay" && topic == "export-fbin" => {
            assay_fbin_export::run(rest)
        }
        [command, topic, rest @ ..] if command == "assay" && topic == "gdelt-rows" => {
            assay_gdelt_rows::run(rest)
        }
        [command, topic, rest @ ..] if command == "assay" && topic == "stream-fbin" => {
            assay_stream_fbin::run(rest)
        }
        [command, topic, rest @ ..] if command == "ward" && topic == "guard-validate" => {
            ward_guard_validation::run(rest)
        }
        [command, topic, rest @ ..] if command == "oracle" && topic == "sufficiency-validate" => {
            oracle_sufficiency_validation::run(rest)
        }
        [command, rest @ ..] if command == "htap-validate" => htap_validation::run(rest),
        [command, topic, rest @ ..] if command == "lens" => lens_commands::run(topic, rest),
        [command, topic, rest @ ..] if command == "panel" => panel_commands::run(topic, rest),
        [command, rest @ ..] if command == "summarize" => summarize_command::run(rest),
        [command, topic, rest @ ..] if command == "readback" && topic == "ledger" => {
            anneal_ledger_readback::run(rest)
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
        [command, topic, vault_flag, vault]
            if command == "readback" && topic == "time-index" && vault_flag == "--vault" =>
        {
            timetravel_readback::readback_time_index(Path::new(vault))
        }
        [command, topic, sub_id, vault_flag, vault]
            if command == "readback" && topic == "trigger-audit" && vault_flag == "--vault" =>
        {
            trigger_readback::readback_trigger_audit(Path::new(vault), sub_id)
        }
        [command, topic, vault_flag, vault]
            if command == "readback" && topic == "trigger-fired" && vault_flag == "--vault" =>
        {
            trigger_readback::readback_trigger_fired(Path::new(vault))
        }
        [command, topic, vault_flag, vault, t_flag, t_millis]
            if command == "readback"
                && topic == "as-of"
                && vault_flag == "--vault"
                && t_flag == "--t-millis" =>
        {
            timetravel_readback::readback_as_of(Path::new(vault), t_millis)
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
        [command, vault_flag, vault, last_flag, last]
            if command == "ledger-tail" && vault_flag == "--vault" && last_flag == "--last" =>
        {
            let last = last
                .parse::<usize>()
                .map_err(|error| format!("invalid --last: {error}"))?;
            scan::tail_ledger_vault(Path::new(vault), last)
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
        [command, vault_flag, vault] if command == "resource-status" && vault_flag == "--vault" => {
            resource_status::run_resource_status(
                Path::new(vault),
                resource_status::ResourceStatusFormat::Json,
            )
        }
        [command, vault_flag, vault, metrics_flag]
            if command == "resource-status"
                && vault_flag == "--vault"
                && metrics_flag == "--metrics" =>
        {
            resource_status::run_resource_status(
                Path::new(vault),
                resource_status::ResourceStatusFormat::Metrics,
            )
        }
        [
            command,
            vault_flag,
            vault,
            ops_flag,
            ops,
            value_flag,
            value_bytes,
            cap_flag,
            memtable_cap,
            pin_flag,
            pin_max_age_ms,
        ] if command == "resource-drill"
            && vault_flag == "--vault"
            && ops_flag == "--ops"
            && value_flag == "--value-bytes"
            && cap_flag == "--memtable-cap"
            && pin_flag == "--pin-max-age-ms" =>
        {
            let args = resource_drill::ResourceDrillArgs {
                ops: ops
                    .parse::<u64>()
                    .map_err(|error| format!("invalid --ops: {error}"))?,
                value_bytes: value_bytes
                    .parse::<usize>()
                    .map_err(|error| format!("invalid --value-bytes: {error}"))?,
                memtable_cap: memtable_cap
                    .parse::<usize>()
                    .map_err(|error| format!("invalid --memtable-cap: {error}"))?,
                pin_max_age_ms: pin_max_age_ms
                    .parse::<u64>()
                    .map_err(|error| format!("invalid --pin-max-age-ms: {error}"))?,
            };
            resource_drill::run_resource_drill(Path::new(vault), args)
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
        _ => Err(CliError::usage(usage::usage())),
    }
}
