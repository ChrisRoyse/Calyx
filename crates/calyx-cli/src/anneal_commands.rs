use std::path::Path;

use crate::{
    anneal_ab_log, anneal_autotune_report, anneal_bandit_readback, anneal_deficit_map,
    anneal_frozen_guard_readback, anneal_head_readback, anneal_propose_preview,
    anneal_regression_readback, anneal_replay_readback, anneal_soak_report, anneal_status,
};

pub(crate) fn run(topic: &str, rest: &[String]) -> Result<(), String> {
    match (topic, rest) {
        ("status", [health_flag, vault_flag, vault])
            if health_flag == "--health" && vault_flag == "--vault" =>
        {
            anneal_status::status_health(Path::new(vault))
        }
        ("replay-status", [vault_flag, vault]) if vault_flag == "--vault" => {
            anneal_replay_readback::replay_status(Path::new(vault))
        }
        ("head-status", [kind_flag, kind, vault_flag, vault])
            if kind_flag == "--kind" && vault_flag == "--vault" =>
        {
            anneal_head_readback::head_status(Path::new(vault), kind)
        }
        ("head-status", [vault_flag, vault, kind_flag, kind])
            if vault_flag == "--vault" && kind_flag == "--kind" =>
        {
            anneal_head_readback::head_status(Path::new(vault), kind)
        }
        ("bandit-status", [key_flag, key, vault_flag, vault])
            if key_flag == "--key" && vault_flag == "--vault" =>
        {
            anneal_bandit_readback::bandit_status(Path::new(vault), key)
        }
        ("bandit-status", [vault_flag, vault, key_flag, key])
            if vault_flag == "--vault" && key_flag == "--key" =>
        {
            anneal_bandit_readback::bandit_status(Path::new(vault), key)
        }
        ("ab-log", args) => anneal_ab_log::run(args),
        ("soak-report", args) => anneal_soak_report::run(args),
        ("autotune-report", args) => anneal_autotune_report::run(args),
        ("deficit-map", args) => anneal_deficit_map::run(args),
        ("propose-preview", args) => anneal_propose_preview::run(args),
        ("frozen-guard-report", [artifact_flag, artifact]) if artifact_flag == "--artifact" => {
            anneal_frozen_guard_readback::frozen_guard_report(Path::new(artifact))
        }
        ("regression-report", [artifact_flag, artifact]) if artifact_flag == "--artifact" => {
            anneal_regression_readback::regression_report(Path::new(artifact))
        }
        ("status", [faults_flag, last_flag, last, vault_flag, vault])
            if faults_flag == "--faults" && last_flag == "--last" && vault_flag == "--vault" =>
        {
            anneal_status::status_faults(Path::new(vault), anneal_status::parse_last(last)?)
        }
        _ => Err(format!(
            "unknown anneal command: {topic} {}",
            rest.join(" ")
        )),
    }
}
