use std::process::Command;

use calyx_aster::mvcc::ReadBarrier;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, Result};
use serde::{Deserialize, Serialize};

use super::checksum::{load_barriered_shards, write_shard_status};
use super::{BaseFaultEvent, BaseShard, ShardId};

pub const CALYX_ANNEAL_RESTORE_FAILED: &str = "CALYX_ANNEAL_RESTORE_FAILED";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreCommand {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreConfig {
    pub auto_restore: bool,
    pub command: Option<RestoreCommand>,
}

impl RestoreConfig {
    pub const fn operator_required() -> Self {
        Self {
            auto_restore: false,
            command: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RestoreOutcome {
    OperatorRequired { shard_id: ShardId },
    CommandSucceeded { shard_id: ShardId, program: String },
}

pub fn fail_reads_on_range<C>(vault: &AsterVault<C>, event: &BaseFaultEvent) -> Result<()>
where
    C: Clock,
{
    let shard = event.shard();
    vault.install_read_barrier(ReadBarrier::base_corrupt(
        shard.shard_id.as_str(),
        shard.cf_range.clone(),
    ));
    write_shard_status(
        vault,
        shard,
        true,
        Some(event.actual()),
        event.detected_at(),
    )
    .map(|_| ())
}

pub fn clear_reads_on_range<C>(
    vault: &AsterVault<C>,
    shard: &BaseShard,
    clock: &dyn Clock,
) -> Result<()>
where
    C: Clock,
{
    vault.remove_read_barrier(shard.shard_id.as_str());
    write_shard_status(vault, shard, false, None, clock.now()).map(|_| ())
}

pub fn install_recorded_read_barriers<C>(vault: &AsterVault<C>) -> Result<usize>
where
    C: Clock,
{
    let shards = load_barriered_shards(vault)?;
    let count = shards.len();
    for shard in shards {
        vault.install_read_barrier(ReadBarrier::base_corrupt(
            shard.shard_id.as_str(),
            shard.cf_range,
        ));
    }
    Ok(count)
}

pub fn attempt_restore(shard_id: ShardId, config: &RestoreConfig) -> Result<RestoreOutcome> {
    if !config.auto_restore {
        return Ok(RestoreOutcome::OperatorRequired { shard_id });
    }
    let command = config
        .command
        .as_ref()
        .ok_or_else(|| restore_failed("auto_restore=true but no restore command configured"))?;
    let status = Command::new(&command.program)
        .args(&command.args)
        .status()
        .map_err(|error| restore_failed(format!("launch restore command: {error}")))?;
    if !status.success() {
        return Err(restore_failed(format!(
            "restore command exited with status {status}"
        )));
    }
    Ok(RestoreOutcome::CommandSucceeded {
        shard_id,
        program: command.program.clone(),
    })
}

fn restore_failed(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_RESTORE_FAILED,
        message: message.into(),
        remediation: "restore the base shard from restic/ZFS, then re-run checksum verification",
    }
}
