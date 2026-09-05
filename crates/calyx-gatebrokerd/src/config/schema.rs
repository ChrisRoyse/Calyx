//! On-disk broker configuration schema: the raw deserialized shape before any
//! authority rule has been checked. No value in this module is trusted until
//! [`super::validate`] accepts it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::protocol::{ExecutionRootAlias, RootAlias};

pub(super) const MAX_ROOTS: usize = 32;
pub(super) const MAX_EXECUTION_ROOTS: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerConfig {
    pub schema_version: u16,
    pub socket_path: PathBuf,
    pub journal_path: PathBuf,
    pub worker_user: String,
    pub client_group: String,
    pub unit_prefix: String,
    pub max_active_runs: usize,
    pub max_rpc_frame_bytes: usize,
    pub max_argv_entries: usize,
    pub max_environment_entries: usize,
    pub state: StateConfig,
    pub journal: JournalConfig,
    pub containment: ContainmentConfig,
    pub roots: BTreeMap<RootAlias, RootConfig>,
    pub execution_roots: BTreeMap<ExecutionRootAlias, ExecutionRootConfig>,
}

/// Root-owned persistent namespace from which every mutable broker authority
/// descends. `anchor` remains traversable by the worker while `private` is the
/// root-only journal and recovery namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateConfig {
    pub anchor: PathBuf,
    pub anchor_owner: String,
    pub anchor_mode: String,
    pub private: PathBuf,
    pub private_owner: String,
    pub private_mode: String,
    pub journal_directory: PathBuf,
    pub journal_directory_owner: String,
    pub journal_directory_mode: String,
    pub require_root_owned_path_chain: bool,
    pub require_no_symlinks: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalConfig {
    pub mode: String,
    pub synchronous: String,
    pub foreign_keys: bool,
    pub integrity_check_on_start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainmentConfig {
    pub system_manager: bool,
    pub cgroup_version: u8,
    pub delegate: bool,
    pub bind_units_to_broker: bool,
    pub require_pidfd_owner: bool,
    pub require_held_cgroup_fd: bool,
    pub allow_user_manager: bool,
    pub allow_same_uid_stage: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootConfig {
    pub common_ancestor: PathBuf,
    pub shared: PathBuf,
    pub private: PathBuf,
    pub shared_owner: String,
    pub shared_mode: String,
    pub private_owner: String,
    pub private_mode: String,
    pub published_mode: String,
    pub require_same_mount: bool,
    pub require_rename_noreplace: bool,
    pub require_opaque_file_handles: bool,
    pub allow_existing_object_adoption: bool,
}

/// A read-only path capability used to resolve `ExecStage.cwd` through a held
/// directory descriptor. It never grants create, rename, or delete authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRootConfig {
    pub path: PathBuf,
    pub expected_owner: String,
    pub expected_mode: String,
    pub read_only: bool,
    pub require_openat2: bool,
    pub require_resolve_beneath: bool,
    pub require_no_symlinks: bool,
    pub require_no_magiclinks: bool,
}
