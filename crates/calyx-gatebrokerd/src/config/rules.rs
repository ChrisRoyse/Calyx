//! Authority rules for broker configuration: cross-field policy that decides
//! whether a raw [`BrokerConfig`] may become a [`ValidatedConfig`]. Every rule
//! fails closed; nothing here mutates or defaults a value.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::protocol::{
    ExecutionRootAlias, MAX_ARGV_ITEMS, MAX_ENV_ITEMS, MAX_FRAME_BYTES, PROTOCOL_VERSION, RootAlias,
};

use super::ConfigError;
use super::primitives::{
    bounded_nonzero, contains, invalid, parse_mode, require_eq, require_false, require_true,
    validate_absolute_dir, validate_absolute_file, validate_account, validate_unit_prefix,
};
use super::schema::{
    BrokerConfig, ContainmentConfig, ExecutionRootConfig, JournalConfig, MAX_EXECUTION_ROOTS,
    MAX_ROOTS, RootConfig, StateConfig,
};
use super::validated::{
    ValidatedConfig, ValidatedExecutionRootConfig, ValidatedRootConfig, ValidatedStateConfig,
};

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
    require_eq("journal.mode", &journal.mode, "mvcc")?;
    require_eq("journal.synchronous", &journal.synchronous, "full")?;
    require_true("journal.foreign_keys", journal.foreign_keys)?;
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
