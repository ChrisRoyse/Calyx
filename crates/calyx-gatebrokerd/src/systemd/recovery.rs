use super::account::{
    cgroup_contains, verify_worker_account_identity, verify_worker_manager_absent,
    worker_process_locations,
};
use super::cgroup::validate_control_group;
use super::manager::{
    property_has_unit, show_properties, verify_broker_unit, verify_fixed_slice_contract,
    verify_systemd_contract,
};
use super::validation::valid_unit_component;
use super::*;

pub(super) fn recovery_error(error: SystemdError) -> SystemdError {
    match error {
        SystemdError::RecoveryRequired { .. } => error,
        other => SystemdError::RecoveryRequired {
            detail: other.to_string(),
        },
    }
}

pub(super) fn validate_recorded_identity(identity: &CgroupIdentity) -> Result<(), SystemdError> {
    validate_control_group(identity.control_group.as_str()).map_err(recovery_error)?;
    if identity.inode == 0 {
        return Err(SystemdError::RecoveryRequired {
            detail: format!(
                "recorded cgroup identity is zero: path={} dev={} ino={}",
                identity.control_group.as_str(),
                identity.device,
                identity.inode
            ),
        });
    }
    Ok(())
}

pub(super) fn validate_worker_identity(
    expected: &WorkerIdentity,
) -> Result<WorkerAccount, SystemdError> {
    if expected.uid == 0 {
        return Err(SystemdError::RecoveryRequired {
            detail: "recorded worker uid is zero".into(),
        });
    }
    let account =
        verify_worker_account_identity(&expected.user, expected.uid).map_err(recovery_error)?;
    verify_worker_manager_absent(&account, &expected.user).map_err(recovery_error)?;
    Ok(account)
}

pub(super) fn ensure_workers_within(
    uid: u32,
    boundary: &str,
) -> Result<Vec<(u32, String)>, SystemdError> {
    let locations = worker_process_locations(uid).map_err(recovery_error)?;
    let outside: Vec<_> = locations
        .iter()
        .filter(|(_, control_group)| !cgroup_contains(boundary, control_group))
        .cloned()
        .collect();
    if !outside.is_empty() {
        return Err(SystemdError::RecoveryRequired {
            detail: format!(
                "dedicated worker uid {uid} has processes outside fixed boundary {boundary}: {outside:?}"
            ),
        });
    }
    Ok(locations)
}

pub(super) fn open_recorded_group(
    root: &CgroupRoot,
    identity: &CgroupIdentity,
) -> Result<Option<CgroupAuthority>, SystemdError> {
    let group = root
        .open_group_optional(identity.control_group.as_str())
        .map_err(recovery_error)?;
    if let Some(group) = &group {
        group.verify_identity(identity)?;
    }
    Ok(group)
}

const RECOVERY_SERVICE_PROPERTIES: &[&str] = &[
    "LoadState",
    "Id",
    "InvocationID",
    "ControlGroup",
    "ActiveState",
    "User",
    "Slice",
    "BindsTo",
    "After",
];

pub(super) fn validate_service_readback(
    values: &BTreeMap<String, String>,
    unit_name: &str,
    invocation_id: &str,
    service: &CgroupIdentity,
    worker: &WorkerIdentity,
) -> Result<bool, SystemdError> {
    let load_state = values
        .get("LoadState")
        .map(String::as_str)
        .unwrap_or_default();
    if load_state == "not-found" {
        return Ok(false);
    }
    if load_state != "loaded"
        || values.get("Id").map(String::as_str) != Some(unit_name)
        || values.get("InvocationID").map(String::as_str) != Some(invocation_id)
        || values.get("ControlGroup").map(String::as_str) != Some(service.control_group.as_str())
        || values.get("User").map(String::as_str) != Some(worker.user.as_str())
        || values.get("Slice").map(String::as_str) != Some(STAGE_SLICE_NAME)
        || !property_has_unit(values, "BindsTo", BROKER_UNIT_NAME)
        || !property_has_unit(values, "After", BROKER_UNIT_NAME)
    {
        return Err(SystemdError::RecoveryRequired {
            detail: format!("recorded service identity/policy mismatch: {values:?}"),
        });
    }
    Ok(true)
}

pub(super) fn validate_slice_readback(
    values: &BTreeMap<String, String>,
    group_is_open: bool,
    group_is_populated: bool,
) -> Result<(), SystemdError> {
    let control_group = values
        .get("ControlGroup")
        .map(String::as_str)
        .unwrap_or_default();
    if group_is_open && group_is_populated && control_group != STAGE_SLICE_CONTROL_GROUP {
        return Err(SystemdError::RecoveryRequired {
            detail: format!(
                "populated fixed-slice descriptor is not the cgroup published by PID1: {values:?}"
            ),
        });
    }
    if !control_group.is_empty() && control_group != STAGE_SLICE_CONTROL_GROUP {
        return Err(SystemdError::RecoveryRequired {
            detail: format!("fixed slice cgroup changed: {values:?}"),
        });
    }
    Ok(())
}

pub(super) fn drain_held_groups(
    service: Option<&CgroupAuthority>,
    slice: Option<&CgroupAuthority>,
    account: &WorkerAccount,
    worker_user: &str,
) -> Result<RecoveryOutcome, SystemdError> {
    let service_was_populated = service
        .map(CgroupAuthority::population)
        .transpose()
        .map_err(recovery_error)?
        == Some(CgroupPopulation::Populated);
    let slice_was_populated = slice
        .map(CgroupAuthority::population)
        .transpose()
        .map_err(recovery_error)?
        == Some(CgroupPopulation::Populated);
    if let Some(service) = service {
        service.kill_if_populated().map_err(recovery_error)?;
    }
    if let Some(slice) = slice {
        slice.kill_if_populated().map_err(recovery_error)?;
    }
    if let Some(service) = service {
        service.prove_empty(DRAIN_TIMEOUT).map_err(recovery_error)?;
    }
    if let Some(slice) = slice {
        slice.prove_empty(DRAIN_TIMEOUT).map_err(recovery_error)?;
    }
    verify_worker_manager_absent(account, worker_user).map_err(recovery_error)?;
    let remaining = worker_process_locations(account.uid).map_err(recovery_error)?;
    if !remaining.is_empty() {
        return Err(SystemdError::RecoveryRequired {
            detail: format!(
                "worker processes remain after exact cgroup drain: uid={} locations={remaining:?}",
                account.uid
            ),
        });
    }
    Ok(if service_was_populated || slice_was_populated {
        RecoveryOutcome::Killed
    } else {
        RecoveryOutcome::AbsentOrEmpty
    })
}

#[derive(Debug, Clone, Copy)]
struct RecoveryReport {
    outcome: RecoveryOutcome,
    stage_identity_present: bool,
}

fn recover_recorded_stage_report(
    unit_name: &UnitName,
    expected_invocation_id: &InvocationId,
    service_identity: &CgroupIdentity,
    slice_identity: &CgroupIdentity,
    worker_identity: &WorkerIdentity,
) -> Result<RecoveryReport, SystemdError> {
    let unit_name = unit_name.as_str();
    let invocation_id = expected_invocation_id.as_str();
    if !valid_unit_component(unit_name, ".service")
        || invocation_id.len() != 32
        || !invocation_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(SystemdError::RecoveryRequired {
            detail: "recorded unit/invocation identity is malformed".into(),
        });
    }
    validate_recorded_identity(service_identity)?;
    validate_recorded_identity(slice_identity)?;
    if slice_identity.control_group.as_str() != STAGE_SLICE_CONTROL_GROUP {
        return Err(SystemdError::RecoveryRequired {
            detail: format!(
                "recorded slice is not fixed boundary {STAGE_SLICE_CONTROL_GROUP}: {}",
                slice_identity.control_group.as_str()
            ),
        });
    }
    let expected_prefix = format!("{STAGE_SLICE_CONTROL_GROUP}/");
    if !service_identity
        .control_group
        .as_str()
        .strip_prefix(&expected_prefix)
        .is_some_and(|leaf| !leaf.is_empty() && !leaf.contains('/'))
    {
        return Err(SystemdError::RecoveryRequired {
            detail: format!(
                "recorded service cgroup is not an immediate fixed-slice child: {}",
                service_identity.control_group.as_str()
            ),
        });
    }

    verify_systemd_contract().map_err(recovery_error)?;
    verify_broker_unit(BROKER_UNIT_NAME).map_err(recovery_error)?;
    verify_fixed_slice_contract().map_err(recovery_error)?;
    let account = validate_worker_identity(worker_identity)?;
    let root = CgroupRoot::open().map_err(recovery_error)?;

    let first_service =
        show_properties(unit_name, RECOVERY_SERVICE_PROPERTIES).map_err(recovery_error)?;
    let first_loaded = validate_service_readback(
        &first_service,
        unit_name,
        invocation_id,
        service_identity,
        worker_identity,
    )?;

    // Both authorities are opened relative to the already verified cgroup2
    // root and compared to journaled dev+ino before any destructive action.
    let service = open_recorded_group(&root, service_identity)?;
    let slice = open_recorded_group(&root, slice_identity)?;
    if service.is_some() && slice.is_none() {
        return Err(SystemdError::RecoveryRequired {
            detail: "recorded service cgroup exists but its fixed parent slice does not".into(),
        });
    }
    if first_loaded && service.is_none() {
        return Err(SystemdError::RecoveryRequired {
            detail: "same recorded invocation is loaded but its persisted cgroup is absent".into(),
        });
    }

    // Close the show->open replacement gap: after both descriptors are held,
    // re-read PID1 and require the same unit/invocation/path publication.
    let second_service =
        show_properties(unit_name, RECOVERY_SERVICE_PROPERTIES).map_err(recovery_error)?;
    let second_loaded = validate_service_readback(
        &second_service,
        unit_name,
        invocation_id,
        service_identity,
        worker_identity,
    )?;
    if first_loaded != second_loaded {
        return Err(SystemdError::RecoveryRequired {
            detail: format!(
                "service load identity changed while cgroup descriptors were acquired: before={first_service:?} after={second_service:?}"
            ),
        });
    }
    let second_slice = verify_fixed_slice_contract().map_err(recovery_error)?;
    let slice_populated = slice
        .as_ref()
        .map(CgroupAuthority::population)
        .transpose()
        .map_err(recovery_error)?
        == Some(CgroupPopulation::Populated);
    validate_slice_readback(&second_slice, slice.is_some(), slice_populated)?;
    ensure_workers_within(account.uid, STAGE_SLICE_CONTROL_GROUP)?;

    let stage_identity_present = first_loaded || service.is_some() || slice_populated;
    let outcome = drain_held_groups(
        service.as_ref(),
        slice.as_ref(),
        &account,
        &worker_identity.user,
    )?;
    Ok(RecoveryReport {
        outcome,
        stage_identity_present,
    })
}

pub fn recover_recorded_stage(
    unit_name: &UnitName,
    expected_invocation_id: &InvocationId,
    service_identity: &CgroupIdentity,
    slice_identity: &CgroupIdentity,
    worker_identity: &WorkerIdentity,
) -> Result<RecoveryOutcome, SystemdError> {
    recover_recorded_stage_report(
        unit_name,
        expected_invocation_id,
        service_identity,
        slice_identity,
        worker_identity,
    )
    .map(|report| report.outcome)
}

pub fn audit_terminal_recorded_stage(
    unit_name: &UnitName,
    expected_invocation_id: &InvocationId,
    service_identity: &CgroupIdentity,
    slice_identity: &CgroupIdentity,
    worker_identity: &WorkerIdentity,
) -> Result<(), SystemdError> {
    let report = recover_recorded_stage_report(
        unit_name,
        expected_invocation_id,
        service_identity,
        slice_identity,
        worker_identity,
    )?;
    if report.stage_identity_present {
        return Err(SystemdError::TerminalStageInvariant {
            detail: format!(
                "unit={} invocation={} recovery_outcome={:?}",
                unit_name.as_str(),
                expected_invocation_id.as_str(),
                report.outcome
            ),
        });
    }
    Ok(())
}

pub fn recover_worker_boundary(
    worker_identity: &WorkerIdentity,
) -> Result<RecoveryOutcome, SystemdError> {
    verify_systemd_contract().map_err(recovery_error)?;
    verify_broker_unit(BROKER_UNIT_NAME).map_err(recovery_error)?;
    let first_slice = verify_fixed_slice_contract().map_err(recovery_error)?;
    let account = validate_worker_identity(worker_identity)?;
    ensure_workers_within(account.uid, STAGE_SLICE_CONTROL_GROUP)?;
    let root = CgroupRoot::open().map_err(recovery_error)?;
    let slice = root
        .open_group_optional(STAGE_SLICE_CONTROL_GROUP)
        .map_err(recovery_error)?;
    let slice_populated = slice
        .as_ref()
        .map(CgroupAuthority::population)
        .transpose()
        .map_err(recovery_error)?
        == Some(CgroupPopulation::Populated);
    validate_slice_readback(&first_slice, slice.is_some(), slice_populated)?;

    // Bind the dynamically discovered fixed boundary to a second PID1 read
    // before using cgroup.kill. This path intentionally has no stage identity:
    // it is for journal INTENT recovery before publication was committed.
    let second_slice = verify_fixed_slice_contract().map_err(recovery_error)?;
    validate_slice_readback(&second_slice, slice.is_some(), slice_populated)?;
    if first_slice.get("ControlGroup") != second_slice.get("ControlGroup") {
        return Err(SystemdError::RecoveryRequired {
            detail: format!(
                "fixed slice publication changed while its descriptor was acquired: before={first_slice:?} after={second_slice:?}"
            ),
        });
    }
    ensure_workers_within(account.uid, STAGE_SLICE_CONTROL_GROUP)?;
    drain_held_groups(None, slice.as_ref(), &account, &worker_identity.user)
}
