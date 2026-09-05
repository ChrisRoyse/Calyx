use super::validation::{io_error, trusted_executable};
use super::*;

pub(super) fn manager_command(path: &'static str) -> Command {
    let mut command = Command::new(path);
    command.env_clear();
    command.env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin");
    command.env("LC_ALL", "C");
    command.env("SYSTEMD_COLORS", "0");
    command.env("SYSTEMD_URLIFY", "0");
    command
}

pub(super) fn command_output(
    mut command: Command,
    operation: &'static str,
) -> Result<String, SystemdError> {
    let output = command
        .output()
        .map_err(|source| io_error(operation, source))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(SystemdError::Manager {
            operation,
            detail: format!(
                "status={} stdout={} stderr={}",
                output.status,
                stdout.trim(),
                stderr.trim()
            ),
        });
    }
    Ok(stdout.into_owned())
}

pub(super) fn verify_systemd_contract() -> Result<(), SystemdError> {
    trusted_executable(SYSTEMD_RUN, false)?;
    trusted_executable(SYSTEMCTL, false)?;
    let mut command = manager_command(SYSTEMCTL);
    command.arg("--version");
    let output = command_output(command, "query systemd version")?;
    let version = output
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("systemd "))
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| SystemdError::Manager {
            operation: "query systemd version",
            detail: format!("malformed version output: {output:?}"),
        })?;
    if version < MINIMUM_SYSTEMD_VERSION {
        return Err(SystemdError::Manager {
            operation: "query systemd version",
            detail: format!("systemd {version} is below required {MINIMUM_SYSTEMD_VERSION}"),
        });
    }
    Ok(())
}

pub(super) fn show_properties(
    unit: &str,
    properties: &[&str],
) -> Result<BTreeMap<String, String>, SystemdError> {
    let mut command = manager_command(SYSTEMCTL);
    command.args(["--system", "--no-pager", "show", unit]);
    for property in properties {
        command.arg(format!("--property={property}"));
    }
    let raw = command_output(command, "inspect system unit")?;
    let mut values = BTreeMap::new();
    for line in raw.lines() {
        if let Some((key, value)) = line.split_once('=') {
            values.insert(key.to_owned(), value.to_owned());
        }
    }
    Ok(values)
}

pub(super) fn verify_broker_unit(unit: &str) -> Result<(), SystemdError> {
    let values = show_properties(unit, &["LoadState", "Id", "ActiveState"])?;
    if values.get("LoadState").map(String::as_str) != Some("loaded")
        || values.get("Id").map(String::as_str) != Some(unit)
        || values.get("ActiveState").map(String::as_str) != Some("active")
    {
        return Err(SystemdError::Manager {
            operation: "verify broker unit",
            detail: format!("unit={unit} properties={values:?}"),
        });
    }
    Ok(())
}

pub(super) fn property_has_unit(
    values: &BTreeMap<String, String>,
    property: &str,
    unit: &str,
) -> bool {
    values
        .get(property)
        .is_some_and(|value| value.split_whitespace().any(|candidate| candidate == unit))
}

pub(super) fn verify_fixed_slice_contract() -> Result<BTreeMap<String, String>, SystemdError> {
    let values = show_properties(
        STAGE_SLICE_NAME,
        &[
            "LoadState",
            "Id",
            "ActiveState",
            "ControlGroup",
            "Transient",
            "StopWhenUnneeded",
            "BindsTo",
            "Before",
        ],
    )?;
    let control_group = values
        .get("ControlGroup")
        .map(String::as_str)
        .unwrap_or_default();
    if values.get("LoadState").map(String::as_str) != Some("loaded")
        || values.get("Id").map(String::as_str) != Some(STAGE_SLICE_NAME)
        || values.get("Transient").map(String::as_str) != Some("no")
        || values.get("StopWhenUnneeded").map(String::as_str) != Some("yes")
        || !property_has_unit(&values, "BindsTo", BROKER_UNIT_NAME)
        || !property_has_unit(&values, "Before", BROKER_UNIT_NAME)
        || (!control_group.is_empty() && control_group != STAGE_SLICE_CONTROL_GROUP)
    {
        return Err(SystemdError::Manager {
            operation: "verify fixed stage slice",
            detail: format!(
                "expected installed {STAGE_SLICE_NAME} BindsTo/Before {BROKER_UNIT_NAME}, StopWhenUnneeded=yes, Transient=no, cgroup={STAGE_SLICE_CONTROL_GROUP}; properties={values:?}"
            ),
        });
    }
    Ok(values)
}
