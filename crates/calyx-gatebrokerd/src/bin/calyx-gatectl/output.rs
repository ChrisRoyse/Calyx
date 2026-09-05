use calyx_gatebrokerd::protocol::{ProfileName, Response, ResponseOutcome, StableCode};

use crate::CliError;
use crate::fd_pipe::TokenWriter;

pub(super) fn render(
    outcome: ResponseOutcome,
    profile: &ProfileName,
    token_output: Option<TokenWriter>,
) -> Result<(serde_json::Value, i32), CliError> {
    match outcome {
        ResponseOutcome::Error(error) => {
            let code = enum_name(&error.code)?;
            let exit = match error.code {
                StableCode::PermissionDenied => 77,
                StableCode::ConfigInvalid => 78,
                StableCode::InvalidFrame
                | StableCode::ProtocolVersionMismatch
                | StableCode::InvalidRequest => 64,
                _ => 125,
            };
            Ok((
                serde_json::json!({
                    "status": "error",
                    "code": code,
                    "message": error.message.as_str(),
                    "remediation": error.remediation.as_str(),
                    "context": error.context,
                }),
                exit,
            ))
        }
        ResponseOutcome::Ok(Response::RunBegun { run_id, run_token }) => {
            let writer = token_output.ok_or_else(|| {
                CliError::protocol(
                    "broker returned a run token without the required token-output pipe",
                )
            })?;
            writer.write(&run_token)?;
            Ok((
                serde_json::json!({
                    "status": "ok",
                    "run": {
                        "id": run_id,
                        "state": "active",
                        "token_transport": "fd"
                    }
                }),
                0,
            ))
        }
        ResponseOutcome::Ok(response) => {
            if token_output.is_some() {
                return Err(CliError::protocol(
                    "begin-run received a successful response that did not contain run authority",
                ));
            }
            render_success(response, profile)
        }
    }
}

fn render_success(
    response: Response,
    profile: &ProfileName,
) -> Result<(serde_json::Value, i32), CliError> {
    match response {
        Response::Health {
            healthy,
            broker,
            worker,
            storage,
            limits,
        } => Ok((
            serde_json::json!({
                "status": "ok", "healthy": healthy, "profile": profile,
                "broker": broker, "worker": worker, "storage": storage, "limits": limits,
            }),
            0,
        )),
        Response::RunBegun { .. } => Err(CliError::protocol(
            "run-begun response reached the non-secret renderer",
        )),
        Response::ObjectCreated {
            object_id,
            absolute_path,
            root_path,
            root_identity,
            object_identity,
            state,
        } => Ok((
            serde_json::json!({
                "status": "ok", "object": {"id": object_id, "path": absolute_path, "root_path": root_path,
                "root_identity": root_identity, "device": object_identity.device, "inode": object_identity.inode, "state": state}
            }),
            0,
        )),
        Response::StageFinished {
            stage_id,
            unit,
            invocation_id,
            control_group,
            exit_status,
        } => {
            let state = if exit_status == 0 {
                "succeeded"
            } else {
                "failed"
            };
            let status = if exit_status == 0 { "ok" } else { "error" };
            let exit = if (1..=124).contains(&exit_status) {
                exit_status
            } else if exit_status == 0 {
                0
            } else {
                125
            };
            Ok((
                serde_json::json!({
                    "status": status,
                    "code": if exit_status == 0 { serde_json::Value::Null } else { serde_json::json!("STAGE_FAILED") },
                    "stage": {"id": stage_id, "state": state, "unit": unit, "invocation_id": invocation_id,
                    "cgroup": control_group, "exit_code": exit_status}
                }),
                exit,
            ))
        }
        Response::ObjectDeleted { object_id } => Ok((
            serde_json::json!({
                "status": "ok", "object": {"id": object_id, "state": "deleted"}
            }),
            0,
        )),
        Response::RunFinished { run_id, status } => Ok((
            serde_json::json!({
                "status": "ok", "run": {"id": run_id, "state": status}
            }),
            0,
        )),
        Response::RunAborted { run_id } => Ok((
            serde_json::json!({
                "status": "ok", "run": {"id": run_id, "state": "aborted"}
            }),
            0,
        )),
        Response::Inspection {
            run,
            objects,
            stages,
            truncated,
        } => Ok((
            serde_json::json!({
                "status": "ok", "run": run, "objects": objects, "stages": stages, "truncated": truncated
            }),
            0,
        )),
    }
}

fn enum_name<T: serde::Serialize>(value: &T) -> Result<String, CliError> {
    serde_json::to_value(value)
        .map_err(|error| CliError::internal("encode response code", error))?
        .as_str()
        .map(|value| value.to_ascii_uppercase())
        .ok_or_else(|| CliError::protocol("response code did not serialize as a string"))
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
