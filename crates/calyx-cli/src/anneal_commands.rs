use std::path::Path;

use crate::{
    anneal_frozen_guard_readback, anneal_head_readback, anneal_replay_readback, anneal_status,
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
        ("frozen-guard-report", [artifact_flag, artifact]) if artifact_flag == "--artifact" => {
            anneal_frozen_guard_readback::frozen_guard_report(Path::new(artifact))
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
