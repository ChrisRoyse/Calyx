use std::collections::VecDeque;
use std::os::fd::RawFd;

use calyx_gatebrokerd::ids;
use calyx_gatebrokerd::pidfd::process_starttime;
use calyx_gatebrokerd::protocol::*;

use crate::fd_pipe::{TokenReader, TokenWriter};

pub(super) fn build_request(
    profile: &ProfileName,
    command: &str,
    args: &mut VecDeque<String>,
    response_fd: Option<RawFd>,
) -> Result<(Request, Option<TokenWriter>), String> {
    let request_id = ids::request_id().map_err(|error| error.to_string())?;
    let mut token_output = None;
    let request = match command {
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
                    "--token-out-fd" => {
                        let fd = parse(next(args, &option)?, &option)?;
                        if Some(fd) == response_fd {
                            return Err(
                                "--token-out-fd and --response-fd must be distinct pipe endpoints"
                                    .into(),
                            );
                        }
                        if token_output.is_some() {
                            return Err("--token-out-fd may be supplied exactly once".into());
                        }
                        token_output = Some(TokenWriter::new(fd, "--token-out-fd")?);
                    }
                    _ => return Err(format!("unknown begin-run option {option:?}")),
                }
            }
            if token_output.is_none() {
                return Err(
                    "begin-run requires --token-out-fd; run tokens are never written to stdout"
                        .into(),
                );
            }
            let owner_pid = owner_pid.unwrap_or_else(|| unsafe { libc::getppid() as u32 });
            let owner_starttime = match owner_starttime {
                Some(value) => value,
                None => process_starttime(owner_pid).map_err(|error| error.to_string())?,
            };
            Ok(Request::BeginRun(BeginRunRequest {
                request_id,
                profile: profile.clone(),
                owner_pid,
                owner_starttime,
            }))
        }
        "create-object" => {
            let run_id = required_typed(args, "--run-id", RunId::new)?;
            let run_token = token(args, response_fd)?;
            let role = optional_typed(args, "--role", RoleName::new)?
                .unwrap_or(RoleName::new("scratch").expect("literal"));
            let root_alias = required_typed(args, "--root", RootAlias::new)?;
            let leaf = optional_typed(args, "--name", LeafName::new)?;
            if let Some(kind) = take_option(args, "--kind")?
                && kind != "directory"
            {
                return Err("--kind must equal directory".into());
            }
            if let Some(mode) = take_option(args, "--mode")?
                && mode != "0700"
            {
                return Err("--mode must equal 0700".into());
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
            let run_token = token(args, response_fd)?;
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
            let run_token = token(args, response_fd)?;
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
            let run_token = token(args, response_fd)?;
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
            let run_token = token(args, response_fd)?;
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
                Some(token(args, response_fd)?)
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
    }?;
    Ok((request, token_output))
}

fn token(args: &mut VecDeque<String>, response_fd: Option<RawFd>) -> Result<RunToken, String> {
    let raw = take_option(args, "--token-fd")?.ok_or_else(|| {
        "--token-fd is required; tokens are never accepted in argv or env".to_owned()
    })?;
    let fd: RawFd = parse(raw, "--token-fd")?;
    if Some(fd) == response_fd {
        return Err("--token-fd and --response-fd must be distinct pipe endpoints".into());
    }
    let value = TokenReader::new(fd, "--token-fd")?.read()?;
    RunToken::new(value).map_err(|error| error.to_string())
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
    "usage: calyx-gatectl [--socket PATH|--config PATH] [--profile NAME] [--response-fd FD] --json COMMAND ...".into()
}
