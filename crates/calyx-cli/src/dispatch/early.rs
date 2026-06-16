use std::path::Path;

use crate::cli_support::{parse_i32, parse_i64, readback_hex};
use crate::error::CliResult;
use crate::{
    dedup_readback, kernel_health_readback, leapable, manifest_readback, recurrence_readback,
    temporal_readback, vault_tree,
};

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    match args {
        [command, flag, value] if command == "readback" && flag == "--hex" => {
            Some(readback_hex(Path::new(value)))
        }
        [command, flag, value] if command == "readback" && flag == "--vault-tree" => {
            Some(vault_tree::readback_vault_tree(Path::new(value)))
        }
        [command, vault_flag, vault, verify_flag, sqlite]
            if command == "readback"
                && vault_flag == "--vault"
                && verify_flag == "--verify-against" =>
        {
            Some(leapable::readback_dual_write_verify(
                Path::new(vault),
                Path::new(sqlite),
            ))
        }
        [command, vault_flag, vault, show_flag]
            if command == "readback"
                && vault_flag == "--vault"
                && show_flag == "--show-manifest" =>
        {
            Some(leapable::readback_shadow_manifest(Path::new(vault)))
        }
        [command, topic, field_flag, field, vault_flag, vault]
            if command == "readback"
                && topic == "vault-manifest"
                && field_flag == "--field"
                && vault_flag == "--vault" =>
        {
            Some(manifest_readback::readback_vault_manifest_field(
                Path::new(vault),
                field,
            ))
        }
        [command, topic, explain_flag, clock_flag, clock, tz_flag, tz]
            if command == "readback"
                && topic == "temporal_search"
                && explain_flag == "--explain"
                && clock_flag == "--clock-fixed"
                && tz_flag == "--tz-offset" =>
        {
            Some((|| {
                temporal_readback::readback_temporal_search(parse_i64(clock)?, parse_i32(tz)?)
            })())
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
            Some(dedup_readback::readback_dedup_check(
                dedup_readback::DedupReadbackArgs {
                    vault: Path::new(vault),
                    cx_id,
                    slot,
                    tau,
                    near_cos,
                    distinct_cos,
                    vault_id,
                    salt,
                },
            ))
        }
        [command, topic, root_flag, root, kernel_flag, kernel_id]
            if command == "readback"
                && topic == "kernel-health"
                && root_flag == "--root"
                && kernel_flag == "--kernel-id" =>
        {
            Some(kernel_health_readback::readback_kernel_health(
                Path::new(root),
                kernel_id,
            ))
        }
        [command, topic, vault_flag, vault, cx_flag, cx_id]
            if command == "readback"
                && topic == "recurrence-series"
                && vault_flag == "--vault"
                && cx_flag == "--cx-id" =>
        {
            Some(recurrence_readback::readback_recurrence_series(
                Path::new(vault),
                cx_id,
            ))
        }
        [command, topic, rest @ ..] if command == "readback" && topic == "periodic-recall" => {
            Some(recurrence_readback::readback_periodic_recall(rest))
        }
        _ => None,
    }
}
