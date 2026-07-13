#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("calyx-gatectl: Linux is required");
    std::process::exit(125);
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::run() {
        let value = serde_json::json!({
            "status": "error",
            "code": "INVALID_ARGUMENT",
            "message": error,
            "remediation": "Correct the command arguments; no broker mutation was attempted."
        });
        println!("{value}");
        std::process::exit(64);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::VecDeque;
    use std::fs::File;
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};
    use std::path::PathBuf;

    use calyx_gatebrokerd::config::{BrokerConfig, validate};
    use calyx_gatebrokerd::ids;
    use calyx_gatebrokerd::pidfd::process_starttime;
    use calyx_gatebrokerd::protocol::*;
    use calyx_gatebrokerd::transport::SeqpacketConnection;

    struct Global {
        socket: PathBuf,
        profile: ProfileName,
    }

    pub fn run() -> Result<(), String> {
        let arguments = std::env::args_os()
            .skip(1)
            .map(|value| {
                value
                    .into_string()
                    .map_err(|_| "arguments must be valid UTF-8".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (global, command, mut args) = parse_global(arguments)?;
        let request = build_request(&global, &command, &mut args)?;
        let request_id = request.request_id().clone();
        let envelope = RequestEnvelope {
            version: PROTOCOL_VERSION,
            request,
        };
        envelope.validate().map_err(|error| error.to_string())?;
        let bytes = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(format!("encoded request exceeds {MAX_FRAME_BYTES} bytes"));
        }
        let connection = SeqpacketConnection::connect(&global.socket)
            .map_err(|error| format!("connect {}: {error}", global.socket.display()))?;
        let rights: Vec<RawFd> = if matches!(envelope.request, Request::ExecStage(_)) {
            vec![std::io::stdout().as_raw_fd(), std::io::stderr().as_raw_fd()]
        } else {
            Vec::new()
        };
        connection
            .send(&bytes, &rights)
            .map_err(|error| format!("send broker request: {error}"))?;
        let frame = connection
            .recv()
            .map_err(|error| format!("receive broker response: {error}"))?;
        if !frame.rights.is_empty() {
            return Err(format!(
                "broker response unexpectedly carried {} file descriptors",
                frame.rights.len()
            ));
        }
        let response = decode_response(&frame.bytes).map_err(|error| error.to_string())?;
        if response.request_id != request_id {
            return Err(format!(
                "broker response request id mismatch: expected={request_id} actual={}",
                response.request_id
            ));
        }
        let (output, exit) = render(response.outcome, &global.profile)?;
        println!(
            "{}",
            serde_json::to_string(&output).map_err(|error| error.to_string())?
        );
        if exit != 0 {
            std::process::exit(exit);
        }
        Ok(())
    }

    fn parse_global(arguments: Vec<String>) -> Result<(Global, String, VecDeque<String>), String> {
        let mut socket: Option<PathBuf> = None;
        let mut config: Option<PathBuf> = None;
        let mut profile = "default".to_owned();
        let mut command = None;
        let mut remaining = VecDeque::new();
        let mut args = VecDeque::from(arguments);
        while let Some(value) = args.pop_front() {
            if command.is_some() {
                remaining.push_back(value);
                remaining.extend(args);
                break;
            }
            match value.as_str() {
                "--socket" => socket = Some(PathBuf::from(next(&mut args, "--socket")?)),
                "--config" => config = Some(PathBuf::from(next(&mut args, "--config")?)),
                "--profile" => profile = next(&mut args, "--profile")?,
                "--json" => {}
                "-h" | "--help" => return Err(usage()),
                value if value.starts_with('-') => {
                    return Err(format!("unknown global option {value:?}\n{}", usage()));
                }
                value => command = Some(value.to_owned()),
            }
        }
        let command = command.ok_or_else(usage)?;
        if socket.is_some() && config.is_some() {
            return Err("use exactly one of --socket or --config".into());
        }
        let socket = if let Some(path) = socket {
            path
        } else if let Some(path) = config {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("read {}: {error}", path.display()))?;
            let raw: BrokerConfig = toml::from_str(&text)
                .map_err(|error| format!("parse {}: {error}", path.display()))?;
            validate(raw)
                .map_err(|error| format!("validate {}: {error}", path.display()))?
                .raw()
                .socket_path
                .clone()
        } else {
            PathBuf::from("/run/calyx-gatebrokerd/control.sock")
        };
        if !socket.is_absolute() {
            return Err("control socket must be absolute".into());
        }
        let profile = ProfileName::new(profile).map_err(|error| error.to_string())?;
        Ok((Global { socket, profile }, command, remaining))
    }

    fn build_request(
        global: &Global,
        command: &str,
        args: &mut VecDeque<String>,
    ) -> Result<Request, String> {
        let request_id = ids::request_id().map_err(|error| error.to_string())?;
        match command {
            "health" => {
                no_more(args)?;
                Ok(Request::Health(HealthRequest { request_id }))
            }
            "begin-run" => {
                let mut owner_pid = None;
                let mut owner_starttime = None;
                while let Some(option) = args.pop_front() {
                    match option.as_str() {
                        "--owner-pid" => owner_pid = Some(parse(next(args, &option)?, &option)?),
                        "--owner-starttime" => {
                            owner_starttime = Some(parse(next(args, &option)?, &option)?)
                        }
                        _ => return Err(format!("unknown begin-run option {option:?}")),
                    }
                }
                let owner_pid = owner_pid.unwrap_or_else(|| unsafe { libc::getppid() as u32 });
                let owner_starttime = match owner_starttime {
                    Some(value) => value,
                    None => process_starttime(owner_pid).map_err(|error| error.to_string())?,
                };
                Ok(Request::BeginRun(BeginRunRequest {
                    request_id,
                    profile: global.profile.clone(),
                    owner_pid,
                    owner_starttime,
                }))
            }
            "create-object" => {
                let run_id = required_typed(args, "--run-id", RunId::new)?;
                let run_token = token(args)?;
                let role = optional_typed(args, "--role", RoleName::new)?
                    .unwrap_or(RoleName::new("scratch").expect("literal"));
                let root_alias = required_typed(args, "--root", RootAlias::new)?;
                let leaf = optional_typed(args, "--name", LeafName::new)?;
                if let Some(kind) = take_option(args, "--kind")? {
                    if kind != "directory" {
                        return Err("--kind must equal directory".into());
                    }
                }
                if let Some(mode) = take_option(args, "--mode")? {
                    if mode != "0700" {
                        return Err("--mode must equal 0700".into());
                    }
                }
                no_more(args)?;
                Ok(Request::CreateObject(CreateObjectRequest {
                    request_id,
                    run_id,
                    run_token,
                    role,
                    root_alias,
                    leaf,
                }))
            }
            "exec-stage" => {
                let separator = args
                    .iter()
                    .position(|value| value == "--")
                    .ok_or_else(|| "exec-stage requires -- followed by argv".to_owned())?;
                let mut argv = args.split_off(separator);
                argv.pop_front();
                let run_id = required_typed(args, "--run-id", RunId::new)?;
                let run_token = token(args)?;
                let label = optional_typed(args, "--label", StageLabel::new)?
                    .unwrap_or(StageLabel::new("stage").expect("literal"));
                let cwd_root = required_typed(args, "--cwd-root", ExecutionRootAlias::new)?;
                let cwd = optional_typed(args, "--cwd", RelativePath::new)?
                    .unwrap_or(RelativePath::new(".").expect("literal"));
                let mut env = Vec::new();
                while let Some(assignment) = take_option(args, "--env")? {
                    let (name, value) = assignment
                        .split_once('=')
                        .ok_or_else(|| "--env requires NAME=VALUE".to_owned())?;
                    env.push(EnvEntry {
                        name: EnvName::new(name).map_err(|error| error.to_string())?,
                        value: EnvValue::new(value).map_err(|error| error.to_string())?,
                    });
                }
                no_more(args)?;
                let argv = argv
                    .into_iter()
                    .map(|value| ArgValue::new(value).map_err(|error| error.to_string()))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Request::ExecStage(ExecStageRequest {
                    request_id,
                    run_id,
                    run_token,
                    label,
                    cwd_root,
                    cwd,
                    argv,
                    env,
                }))
            }
            "delete-object" => {
                let run_id = required_typed(args, "--run-id", RunId::new)?;
                let run_token = token(args)?;
                let object_id = required_typed(args, "--object-id", ObjectId::new)?;
                no_more(args)?;
                Ok(Request::DeleteObject(DeleteObjectRequest {
                    request_id,
                    run_id,
                    run_token,
                    object_id,
                }))
            }
            "finish-run" => {
                let run_id = required_typed(args, "--run-id", RunId::new)?;
                let run_token = token(args)?;
                let intended_status = match take_option(args, "--status")?.as_deref() {
                    None | Some("succeeded") => RunStatus::Succeeded,
                    Some("failed") => RunStatus::Failed,
                    Some(value) => return Err(format!("unsupported finish status {value:?}")),
                };
                no_more(args)?;
                Ok(Request::FinishRun(FinishRunRequest {
                    request_id,
                    run_id,
                    run_token,
                    intended_status,
                }))
            }
            "abort-run" => {
                let run_id = required_typed(args, "--run-id", RunId::new)?;
                let run_token = token(args)?;
                let reason = take_option(args, "--reason")?
                    .unwrap_or_else(|| "controller requested abort".into());
                no_more(args)?;
                Ok(Request::AbortRun(AbortRunRequest {
                    request_id,
                    run_id,
                    run_token,
                    reason: ReasonText::new(reason).map_err(|error| error.to_string())?,
                }))
            }
            "inspect" => {
                let run_id = optional_typed(args, "--run-id", RunId::new)?;
                let run_token = if run_id.is_some() {
                    Some(token(args)?)
                } else {
                    None
                };
                no_more(args)?;
                Ok(Request::Inspect(InspectRequest {
                    request_id,
                    run_id,
                    run_token,
                }))
            }
            _ => Err(format!("unknown command {command:?}\n{}", usage())),
        }
    }

    fn token(args: &mut VecDeque<String>) -> Result<RunToken, String> {
        let raw = take_option(args, "--token-fd")?.ok_or_else(|| {
            "--token-fd is required; tokens are never accepted in argv or env".to_owned()
        })?;
        let fd: RawFd = parse(raw, "--token-fd")?;
        if !(3..=1024).contains(&fd) {
            return Err("--token-fd must be between 3 and 1024".into());
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let mut bytes = Vec::new();
        file.by_ref()
            .take(66)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read --token-fd: {error}"))?;
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
        }
        if bytes.len() != 64 || !bytes.iter().all(u8::is_ascii_hexdigit) {
            return Err(
                "--token-fd must contain exactly 64 hexadecimal bytes and optional newline".into(),
            );
        }
        let value = String::from_utf8(bytes).map_err(|error| error.to_string())?;
        RunToken::new(value).map_err(|error| error.to_string())
    }

    fn render(
        outcome: ResponseOutcome,
        profile: &ProfileName,
    ) -> Result<(serde_json::Value, i32), String> {
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
            ResponseOutcome::Ok(response) => match response {
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
                Response::RunBegun { run_id, run_token } => Ok((
                    serde_json::json!({
                        "status": "ok", "run": {"id": run_id, "token": run_token, "state": "active"}
                    }),
                    0,
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
                        serde_json::json!({"status": status, "code": if exit_status == 0 { serde_json::Value::Null } else { serde_json::json!("STAGE_FAILED") },
                        "stage": {"id": stage_id, "state": state, "unit": unit, "invocation_id": invocation_id,
                        "cgroup": control_group, "exit_code": exit_status}}),
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
            },
        }
    }

    fn enum_name<T: serde::Serialize>(value: &T) -> Result<String, String> {
        serde_json::to_value(value)
            .map_err(|error| error.to_string())?
            .as_str()
            .map(|value| value.to_ascii_uppercase())
            .ok_or_else(|| "enum did not serialize as a string".into())
    }

    fn required_typed<T, F>(
        args: &mut VecDeque<String>,
        name: &str,
        constructor: F,
    ) -> Result<T, String>
    where
        F: FnOnce(String) -> Result<T, ProtocolError>,
    {
        let value = take_option(args, name)?.ok_or_else(|| format!("{name} is required"))?;
        constructor(value).map_err(|error| error.to_string())
    }

    fn optional_typed<T, F>(
        args: &mut VecDeque<String>,
        name: &str,
        constructor: F,
    ) -> Result<Option<T>, String>
    where
        F: FnOnce(String) -> Result<T, ProtocolError>,
    {
        take_option(args, name)?
            .map(constructor)
            .transpose()
            .map_err(|error| error.to_string())
    }

    fn take_option(args: &mut VecDeque<String>, name: &str) -> Result<Option<String>, String> {
        let Some(index) = args.iter().position(|value| value == name) else {
            return Ok(None);
        };
        args.remove(index);
        args.remove(index)
            .map(Some)
            .ok_or_else(|| format!("{name} requires a value"))
    }

    fn next(args: &mut VecDeque<String>, option: &str) -> Result<String, String> {
        args.pop_front()
            .ok_or_else(|| format!("{option} requires a value"))
    }

    fn parse<T: std::str::FromStr>(value: String, option: &str) -> Result<T, String> {
        value
            .parse()
            .map_err(|_| format!("{option} has invalid value {value:?}"))
    }

    fn no_more(args: &VecDeque<String>) -> Result<(), String> {
        if args.is_empty() {
            Ok(())
        } else {
            Err(format!("unexpected arguments: {args:?}"))
        }
    }

    fn usage() -> String {
        "usage: calyx-gatectl [--socket PATH|--config PATH] [--profile NAME] --json COMMAND ..."
            .into()
    }
}
