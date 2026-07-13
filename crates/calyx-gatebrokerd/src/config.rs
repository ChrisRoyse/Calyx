use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::protocol::{
    ExecutionRootAlias, MAX_ARGV_ITEMS, MAX_ENV_ITEMS, MAX_FRAME_BYTES, PROTOCOL_VERSION, RootAlias,
};

const MAX_ROOTS: usize = 32;
const MAX_EXECUTION_ROOTS: usize = 32;

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
    pub trusted_schema: bool,
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

#[derive(Debug, Clone)]
pub struct ValidatedConfig {
    raw: BrokerConfig,
    state: ValidatedStateConfig,
    roots: BTreeMap<RootAlias, ValidatedRootConfig>,
    execution_roots: BTreeMap<ExecutionRootAlias, ValidatedExecutionRootConfig>,
}

impl ValidatedConfig {
    pub fn raw(&self) -> &BrokerConfig {
        &self.raw
    }

    pub fn roots(&self) -> &BTreeMap<RootAlias, ValidatedRootConfig> {
        &self.roots
    }

    pub fn state(&self) -> &ValidatedStateConfig {
        &self.state
    }

    pub fn root(&self, alias: &RootAlias) -> Option<&ValidatedRootConfig> {
        self.roots.get(alias)
    }

    pub fn execution_roots(&self) -> &BTreeMap<ExecutionRootAlias, ValidatedExecutionRootConfig> {
        &self.execution_roots
    }

    pub fn execution_root(
        &self,
        alias: &ExecutionRootAlias,
    ) -> Option<&ValidatedExecutionRootConfig> {
        self.execution_roots.get(alias)
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedStateConfig {
    raw: StateConfig,
    anchor_mode: u32,
    private_mode: u32,
    journal_directory_mode: u32,
}

impl ValidatedStateConfig {
    pub fn raw(&self) -> &StateConfig {
        &self.raw
    }

    pub fn anchor_mode(&self) -> u32 {
        self.anchor_mode
    }

    pub fn private_mode(&self) -> u32 {
        self.private_mode
    }

    pub fn journal_directory_mode(&self) -> u32 {
        self.journal_directory_mode
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedRootConfig {
    alias: RootAlias,
    raw: RootConfig,
    shared_mode: u32,
    private_mode: u32,
    published_mode: u32,
}

impl ValidatedRootConfig {
    pub fn alias(&self) -> &RootAlias {
        &self.alias
    }

    pub fn raw(&self) -> &RootConfig {
        &self.raw
    }

    pub fn shared_mode(&self) -> u32 {
        self.shared_mode
    }

    pub fn private_mode(&self) -> u32 {
        self.private_mode
    }

    pub fn published_mode(&self) -> u32 {
        self.published_mode
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedExecutionRootConfig {
    alias: ExecutionRootAlias,
    raw: ExecutionRootConfig,
    expected_mode: u32,
}

impl ValidatedExecutionRootConfig {
    pub fn alias(&self) -> &ExecutionRootAlias {
        &self.alias
    }

    pub fn raw(&self) -> &ExecutionRootConfig {
        &self.raw
    }

    pub fn expected_mode(&self) -> u32 {
        self.expected_mode
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("unsupported schema_version {actual}; expected {expected}")]
    SchemaVersion { expected: u16, actual: u16 },
    #[error("invalid configuration field {field}: {reason}")]
    InvalidField { field: String, reason: String },
    #[error("unsafe configuration field {field}: required {required}")]
    UnsafeSetting {
        field: String,
        required: &'static str,
    },
    #[error("configured authority paths overlap: {first} and {second}")]
    OverlappingPaths { first: PathBuf, second: PathBuf },
}

pub fn validate(config: BrokerConfig) -> Result<ValidatedConfig, ConfigError> {
    if config.schema_version != PROTOCOL_VERSION {
        return Err(ConfigError::SchemaVersion {
            expected: PROTOCOL_VERSION,
            actual: config.schema_version,
        });
    }
    validate_absolute_file("socket_path", &config.socket_path)?;
    validate_absolute_file("journal_path", &config.journal_path)?;
    if config.socket_path == config.journal_path {
        return Err(ConfigError::OverlappingPaths {
            first: config.socket_path.clone(),
            second: config.journal_path.clone(),
        });
    }
    validate_account("worker_user", &config.worker_user)?;
    validate_account("client_group", &config.client_group)?;
    validate_unit_prefix(&config.unit_prefix)?;
    if config.max_active_runs != 1 {
        return Err(ConfigError::UnsafeSetting {
            field: "max_active_runs".into(),
            required: "1 while the broker uses one fixed worker UID",
        });
    }
    if config.max_rpc_frame_bytes != MAX_FRAME_BYTES {
        return invalid(
            "max_rpc_frame_bytes",
            format!("must equal the protocol limit {MAX_FRAME_BYTES}"),
        );
    }
    bounded_nonzero("max_argv_entries", config.max_argv_entries, MAX_ARGV_ITEMS)?;
    bounded_nonzero(
        "max_environment_entries",
        config.max_environment_entries,
        MAX_ENV_ITEMS,
    )?;
    let state = validate_state(&config.state, &config.journal_path)?;
    if contains(&config.state.anchor, &config.socket_path)
        || contains(&config.socket_path, &config.state.anchor)
    {
        return Err(ConfigError::OverlappingPaths {
            first: config.state.anchor.clone(),
            second: config.socket_path.clone(),
        });
    }
    validate_journal(&config.journal)?;
    validate_containment(&config.containment)?;
    if config.roots.is_empty() || config.roots.len() > MAX_ROOTS {
        return invalid("roots", format!("expected 1..={MAX_ROOTS} entries"));
    }

    let mut roots = BTreeMap::new();
    let mut authority_paths: Vec<PathBuf> = Vec::new();
    for (alias, root) in &config.roots {
        let validated = validate_root(alias, root, &config.state)?;
        for candidate in [&root.shared, &root.private] {
            for existing in &authority_paths {
                if contains(existing, candidate) || contains(candidate, existing) {
                    return Err(ConfigError::OverlappingPaths {
                        first: existing.clone(),
                        second: candidate.clone(),
                    });
                }
            }
            authority_paths.push(candidate.clone());
        }
        roots.insert(alias.clone(), validated);
    }

    if config.execution_roots.is_empty() || config.execution_roots.len() > MAX_EXECUTION_ROOTS {
        return invalid(
            "execution_roots",
            format!("expected 1..={MAX_EXECUTION_ROOTS} entries"),
        );
    }
    let mut execution_roots = BTreeMap::new();
    let mut execution_paths: Vec<PathBuf> = Vec::new();
    for (alias, execution_root) in &config.execution_roots {
        let validated = validate_execution_root(alias, execution_root)?;
        for managed_path in &authority_paths {
            if contains(managed_path, &execution_root.path)
                || contains(&execution_root.path, managed_path)
            {
                return Err(ConfigError::OverlappingPaths {
                    first: managed_path.clone(),
                    second: execution_root.path.clone(),
                });
            }
        }
        if contains(&config.state.anchor, &execution_root.path)
            || contains(&execution_root.path, &config.state.anchor)
        {
            return Err(ConfigError::OverlappingPaths {
                first: config.state.anchor.clone(),
                second: execution_root.path.clone(),
            });
        }
        for existing in &execution_paths {
            if contains(existing, &execution_root.path) || contains(&execution_root.path, existing)
            {
                return Err(ConfigError::OverlappingPaths {
                    first: existing.clone(),
                    second: execution_root.path.clone(),
                });
            }
        }
        execution_paths.push(execution_root.path.clone());
        execution_roots.insert(alias.clone(), validated);
    }
    Ok(ValidatedConfig {
        raw: config,
        state,
        roots,
        execution_roots,
    })
}

fn validate_state(
    state: &StateConfig,
    journal_path: &Path,
) -> Result<ValidatedStateConfig, ConfigError> {
    validate_absolute_dir("state.anchor", &state.anchor)?;
    validate_absolute_dir("state.private", &state.private)?;
    validate_absolute_dir("state.journal_directory", &state.journal_directory)?;
    if state.anchor == state.private || !contains(&state.anchor, &state.private) {
        return invalid(
            "state.private",
            "must be strictly beneath the persistent state anchor",
        );
    }
    if !contains(&state.private, &state.journal_directory)
        || state.private == state.journal_directory
    {
        return invalid(
            "state.journal_directory",
            "must be strictly beneath the root-only state.private directory",
        );
    }
    if journal_path.parent() != Some(state.journal_directory.as_path()) {
        return invalid(
            "journal_path",
            "must be directly beneath state.journal_directory",
        );
    }
    require_eq("state.anchor_owner", &state.anchor_owner, "root")?;
    require_eq("state.private_owner", &state.private_owner, "root")?;
    require_eq(
        "state.journal_directory_owner",
        &state.journal_directory_owner,
        "root",
    )?;
    let anchor_mode = parse_mode("state.anchor_mode", &state.anchor_mode)?;
    if anchor_mode != 0o711 {
        return invalid("state.anchor_mode", "must equal 0711");
    }
    let private_mode = parse_mode("state.private_mode", &state.private_mode)?;
    if private_mode != 0o700 {
        return invalid("state.private_mode", "must equal 0700");
    }
    let journal_directory_mode = parse_mode(
        "state.journal_directory_mode",
        &state.journal_directory_mode,
    )?;
    if journal_directory_mode != 0o700 {
        return invalid("state.journal_directory_mode", "must equal 0700");
    }
    require_true(
        "state.require_root_owned_path_chain",
        state.require_root_owned_path_chain,
    )?;
    require_true("state.require_no_symlinks", state.require_no_symlinks)?;
    Ok(ValidatedStateConfig {
        raw: state.clone(),
        anchor_mode,
        private_mode,
        journal_directory_mode,
    })
}

fn validate_journal(journal: &JournalConfig) -> Result<(), ConfigError> {
    require_eq("journal.mode", &journal.mode, "wal")?;
    require_eq("journal.synchronous", &journal.synchronous, "full")?;
    require_true("journal.foreign_keys", journal.foreign_keys)?;
    require_false("journal.trusted_schema", journal.trusted_schema)?;
    require_true(
        "journal.integrity_check_on_start",
        journal.integrity_check_on_start,
    )
}

fn validate_containment(value: &ContainmentConfig) -> Result<(), ConfigError> {
    require_true("containment.system_manager", value.system_manager)?;
    if value.cgroup_version != 2 {
        return invalid("containment.cgroup_version", "must equal 2");
    }
    require_false("containment.delegate", value.delegate)?;
    require_true(
        "containment.bind_units_to_broker",
        value.bind_units_to_broker,
    )?;
    require_true("containment.require_pidfd_owner", value.require_pidfd_owner)?;
    require_true(
        "containment.require_held_cgroup_fd",
        value.require_held_cgroup_fd,
    )?;
    require_false("containment.allow_user_manager", value.allow_user_manager)?;
    require_false(
        "containment.allow_same_uid_stage",
        value.allow_same_uid_stage,
    )
}

fn validate_root(
    alias: &RootAlias,
    root: &RootConfig,
    state: &StateConfig,
) -> Result<ValidatedRootConfig, ConfigError> {
    let prefix = format!("roots.{alias}");
    for (name, path) in [
        ("common_ancestor", &root.common_ancestor),
        ("shared", &root.shared),
        ("private", &root.private),
    ] {
        validate_absolute_dir(&format!("{prefix}.{name}"), path)?;
    }
    if root.common_ancestor != state.anchor {
        return invalid(
            format!("{prefix}.common_ancestor"),
            "must equal state.anchor; managed deletion roots cannot live beneath caller-owned ancestors",
        );
    }
    if !contains(&root.common_ancestor, &root.shared)
        || root.common_ancestor == root.shared
        || !contains(&root.common_ancestor, &root.private)
        || root.common_ancestor == root.private
    {
        return invalid(
            format!("{prefix}.common_ancestor"),
            "must strictly contain both shared and private roots",
        );
    }
    if contains(&root.shared, &root.private) || contains(&root.private, &root.shared) {
        return Err(ConfigError::OverlappingPaths {
            first: root.shared.clone(),
            second: root.private.clone(),
        });
    }
    if contains(&state.private, &root.shared) {
        return invalid(
            format!("{prefix}.shared"),
            "must remain outside the root-only state.private namespace",
        );
    }
    if !contains(&state.private, &root.private) || state.private == root.private {
        return invalid(
            format!("{prefix}.private"),
            "must be strictly beneath the root-only state.private namespace",
        );
    }
    if contains(&state.journal_directory, &root.private)
        || contains(&root.private, &state.journal_directory)
    {
        return Err(ConfigError::OverlappingPaths {
            first: state.journal_directory.clone(),
            second: root.private.clone(),
        });
    }
    validate_account(&format!("{prefix}.shared_owner"), &root.shared_owner)?;
    validate_account(&format!("{prefix}.private_owner"), &root.private_owner)?;
    require_eq(
        &format!("{prefix}.shared_owner"),
        &root.shared_owner,
        "root",
    )?;
    require_eq(
        &format!("{prefix}.private_owner"),
        &root.private_owner,
        "root",
    )?;
    let shared_mode = parse_mode(&format!("{prefix}.shared_mode"), &root.shared_mode)?;
    let private_mode = parse_mode(&format!("{prefix}.private_mode"), &root.private_mode)?;
    let published_mode = parse_mode(&format!("{prefix}.published_mode"), &root.published_mode)?;
    if private_mode != 0o700 {
        return invalid(format!("{prefix}.private_mode"), "must equal 0700");
    }
    if shared_mode != 0o711 {
        return invalid(format!("{prefix}.shared_mode"), "must equal 0711");
    }
    if published_mode != 0o700 {
        return invalid(format!("{prefix}.published_mode"), "must equal 0700");
    }
    require_true(
        &format!("{prefix}.require_same_mount"),
        root.require_same_mount,
    )?;
    require_true(
        &format!("{prefix}.require_rename_noreplace"),
        root.require_rename_noreplace,
    )?;
    require_true(
        &format!("{prefix}.require_opaque_file_handles"),
        root.require_opaque_file_handles,
    )?;
    require_false(
        &format!("{prefix}.allow_existing_object_adoption"),
        root.allow_existing_object_adoption,
    )?;
    Ok(ValidatedRootConfig {
        alias: alias.clone(),
        raw: root.clone(),
        shared_mode,
        private_mode,
        published_mode,
    })
}

fn validate_execution_root(
    alias: &ExecutionRootAlias,
    root: &ExecutionRootConfig,
) -> Result<ValidatedExecutionRootConfig, ConfigError> {
    let prefix = format!("execution_roots.{alias}");
    validate_absolute_dir(&format!("{prefix}.path"), &root.path)?;
    validate_account(&format!("{prefix}.expected_owner"), &root.expected_owner)?;
    let expected_mode = parse_mode(&format!("{prefix}.expected_mode"), &root.expected_mode)?;
    if expected_mode & 0o022 != 0 {
        return invalid(
            format!("{prefix}.expected_mode"),
            "must deny group/other write so the descriptor root cannot be replaced by a peer",
        );
    }
    if expected_mode & 0o005 != 0o005 {
        return invalid(
            format!("{prefix}.expected_mode"),
            "must grant the distinct worker read and directory traversal",
        );
    }
    require_true(&format!("{prefix}.read_only"), root.read_only)?;
    require_true(&format!("{prefix}.require_openat2"), root.require_openat2)?;
    require_true(
        &format!("{prefix}.require_resolve_beneath"),
        root.require_resolve_beneath,
    )?;
    require_true(
        &format!("{prefix}.require_no_symlinks"),
        root.require_no_symlinks,
    )?;
    require_true(
        &format!("{prefix}.require_no_magiclinks"),
        root.require_no_magiclinks,
    )?;
    Ok(ValidatedExecutionRootConfig {
        alias: alias.clone(),
        raw: root.clone(),
        expected_mode,
    })
}

fn validate_absolute_dir(field: &str, path: &Path) -> Result<(), ConfigError> {
    validate_path(field, path)?;
    if path.parent().is_none() {
        return invalid(field, "filesystem root is not a valid authority directory");
    }
    Ok(())
}

fn validate_absolute_file(field: &str, path: &Path) -> Result<(), ConfigError> {
    validate_path(field, path)?;
    if path.file_name().is_none() || path.parent().is_none() {
        return invalid(field, "must include an absolute parent and filename");
    }
    Ok(())
}

fn validate_path(field: &str, path: &Path) -> Result<(), ConfigError> {
    let value = path.to_str().ok_or_else(|| ConfigError::InvalidField {
        field: field.into(),
        reason: "must be valid UTF-8 for the Linux broker".into(),
    })?;
    if !value.starts_with('/')
        || value.contains("//")
        || (value.len() > 1 && value.ends_with('/'))
        || value
            .split('/')
            .skip(1)
            .any(|part| matches!(part, "." | ".."))
    {
        return invalid(
            field,
            "must be a normalized absolute Linux path without traversal",
        );
    }
    Ok(())
}

fn contains(parent: &Path, child: &Path) -> bool {
    child.starts_with(parent)
}

fn parse_mode(field: &str, value: &str) -> Result<u32, ConfigError> {
    if value.len() != 4
        || !value.starts_with('0')
        || !value.bytes().all(|b| matches!(b, b'0'..=b'7'))
    {
        return invalid(field, "must be four octal digits such as 0700");
    }
    u32::from_str_radix(value, 8).map_err(|error| ConfigError::InvalidField {
        field: field.into(),
        reason: error.to_string(),
    })
}

fn validate_account(field: &str, value: &str) -> Result<(), ConfigError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return invalid(field, "must be a nonempty account name of at most 64 bytes");
    }
    Ok(())
}

fn validate_unit_prefix(value: &str) -> Result<(), ConfigError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return invalid("unit_prefix", "contains a forbidden systemd unit character");
    }
    Ok(())
}

fn bounded_nonzero(field: &str, value: usize, maximum: usize) -> Result<(), ConfigError> {
    if value == 0 || value > maximum {
        return invalid(field, format!("expected 1..={maximum}"));
    }
    Ok(())
}

fn require_eq(field: &str, actual: &str, required: &'static str) -> Result<(), ConfigError> {
    if actual != required {
        return Err(ConfigError::UnsafeSetting {
            field: field.into(),
            required,
        });
    }
    Ok(())
}

fn require_true(field: &str, value: bool) -> Result<(), ConfigError> {
    if !value {
        return Err(ConfigError::UnsafeSetting {
            field: field.into(),
            required: "true",
        });
    }
    Ok(())
}

fn require_false(field: &str, value: bool) -> Result<(), ConfigError> {
    if value {
        return Err(ConfigError::UnsafeSetting {
            field: field.into(),
            required: "false",
        });
    }
    Ok(())
}

fn invalid<T>(field: impl Into<String>, reason: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError::InvalidField {
        field: field.into(),
        reason: reason.into(),
    })
}
