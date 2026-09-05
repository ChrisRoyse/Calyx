#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("calyx-gatectl: Linux is required");
    std::process::exit(125);
}

#[cfg(target_os = "linux")]
#[path = "calyx-gatectl/fd_pipe.rs"]
mod fd_pipe;
#[cfg(target_os = "linux")]
#[path = "calyx-gatectl/output.rs"]
mod output;
#[cfg(target_os = "linux")]
#[path = "calyx-gatectl/request.rs"]
mod request;

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct CliError {
    code: &'static str,
    message: String,
    remediation: &'static str,
    exit: i32,
}

#[cfg(target_os = "linux")]
impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_ARGUMENT",
            message: message.into(),
            remediation: "Correct the command arguments; no broker mutation was attempted.",
            exit: 64,
        }
    }

    fn io(operation: &'static str, error: impl std::fmt::Display) -> Self {
        Self {
            code: "LOCAL_IO_FAILED",
            message: format!("{operation}: {error}"),
            remediation: "Inspect the named local file descriptor or socket operation and retry with a fresh pipe and request.",
            exit: 125,
        }
    }

    fn protocol(message: impl Into<String>) -> Self {
        Self {
            code: "BROKER_PROTOCOL_INVALID",
            message: message.into(),
            remediation: "Stop and inspect the broker/client version and structured broker logs; do not infer success.",
            exit: 125,
        }
    }

    fn internal(operation: &'static str, error: impl std::fmt::Display) -> Self {
        Self {
            code: "CLIENT_INTERNAL",
            message: format!("{operation}: {error}"),
            remediation: "Stop and inspect the client build and the named serialization operation.",
            exit: 125,
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "status": "error",
            "code": self.code,
            "message": self.message,
            "remediation": self.remediation,
        })
    }
}

#[cfg(target_os = "linux")]
fn main() {
    std::process::exit(linux::entry());
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::VecDeque;
    use std::os::fd::{AsRawFd, RawFd};
    use std::path::PathBuf;

    use calyx_gatebrokerd::config::{BrokerConfig, validate};
    use calyx_gatebrokerd::protocol::*;
    use calyx_gatebrokerd::transport::SeqpacketConnection;

    use crate::CliError;
    use crate::fd_pipe::PipeWriter;
    use crate::output::render;
    use crate::request::build_request;

    struct Global {
        socket: PathBuf,
        profile: ProfileName,
    }

    pub fn entry() -> i32 {
        let mut arguments = match std::env::args_os()
            .skip(1)
            .map(|value| {
                value
                    .into_string()
                    .map_err(|_| "arguments must be valid UTF-8".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(arguments) => arguments,
            Err(error) => return emit_terminal(None, &CliError::usage(error)),
        };
        let mut response = match take_response_writer(&mut arguments) {
            Ok(response) => response,
            Err(error) => return emit_terminal(None, &CliError::usage(error)),
        };
        match run(arguments, &mut response) {
            Ok(exit) => exit,
            Err(error) => emit_terminal(response.as_mut(), &error),
        }
    }

    fn run(arguments: Vec<String>, response: &mut Option<PipeWriter>) -> Result<i32, CliError> {
        let (global, command, mut args) = parse_global(arguments).map_err(CliError::usage)?;
        if command == "exec-stage" && response.is_none() {
            return Err(CliError::usage(
                "exec-stage requires --response-fd so stage stdout remains exclusively payload data",
            ));
        }
        let response_fd = response.as_ref().map(PipeWriter::raw_fd);
        let (request, token_output) =
            build_request(&global.profile, &command, &mut args, response_fd)
                .map_err(CliError::usage)?;
        let request_id = request.request_id().clone();
        let envelope = RequestEnvelope {
            version: PROTOCOL_VERSION,
            request,
        };
        envelope
            .validate()
            .map_err(|error| CliError::usage(error.to_string()))?;
        let bytes = serde_json::to_vec(&envelope)
            .map_err(|error| CliError::internal("encode request", error))?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(CliError::usage(format!(
                "encoded request exceeds {MAX_FRAME_BYTES} bytes"
            )));
        }
        let connection = SeqpacketConnection::connect(&global.socket)
            .map_err(|error| CliError::io("connect broker control socket", error))?;
        let rights: Vec<RawFd> = if matches!(envelope.request, Request::ExecStage(_)) {
            vec![std::io::stdout().as_raw_fd(), std::io::stderr().as_raw_fd()]
        } else {
            Vec::new()
        };
        connection
            .send(&bytes, &rights)
            .map_err(|error| CliError::io("send broker request", error))?;
        let frame = connection
            .recv()
            .map_err(|error| CliError::io("receive broker response", error))?;
        if !frame.rights.is_empty() {
            return Err(CliError::protocol(format!(
                "broker response unexpectedly carried {} file descriptors",
                frame.rights.len()
            )));
        }
        let broker_response =
            decode_response(&frame.bytes).map_err(|error| CliError::protocol(error.to_string()))?;
        if broker_response.request_id != request_id {
            return Err(CliError::protocol(format!(
                "broker response request id mismatch: expected={request_id} actual={}",
                broker_response.request_id
            )));
        }
        let (output, exit) = render(broker_response.outcome, &global.profile, token_output)?;
        let encoded = serde_json::to_vec(&output)
            .map_err(|error| CliError::internal("encode response JSON", error))?;
        if let Some(response) = response.as_mut() {
            response.write_frame(&encoded)?;
        } else {
            println!("{}", String::from_utf8(encoded).expect("JSON is UTF-8"));
        }
        Ok(exit)
    }

    fn emit_terminal(response: Option<&mut PipeWriter>, error: &CliError) -> i32 {
        let encoded = serde_json::to_vec(&error.json()).expect("error JSON serialization");
        if let Some(response) = response {
            if let Err(write_error) = response.write_frame(&encoded) {
                eprintln!("{}", write_error.json());
                return write_error.exit;
            }
        } else {
            println!("{}", String::from_utf8(encoded).expect("JSON is UTF-8"));
        }
        error.exit
    }

    fn take_response_writer(arguments: &mut Vec<String>) -> Result<Option<PipeWriter>, String> {
        let mut index = 0;
        let mut found = None;
        while index < arguments.len() {
            match arguments[index].as_str() {
                "--response-fd" => {
                    if found.is_some() {
                        return Err("--response-fd may be supplied exactly once".into());
                    }
                    let raw = arguments
                        .get(index + 1)
                        .ok_or_else(|| "--response-fd requires a value".to_owned())?
                        .clone();
                    let fd = parse(raw, "--response-fd")?;
                    arguments.drain(index..=index + 1);
                    found = Some(PipeWriter::new(fd, "--response-fd")?);
                }
                "--socket" | "--config" | "--profile" => index += 2,
                "--json" => index += 1,
                value if value.starts_with('-') => index += 1,
                _ => break,
            }
        }
        Ok(found)
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

    fn next(args: &mut VecDeque<String>, option: &str) -> Result<String, String> {
        args.pop_front()
            .ok_or_else(|| format!("{option} requires a value"))
    }

    fn parse<T: std::str::FromStr>(value: String, option: &str) -> Result<T, String> {
        value
            .parse()
            .map_err(|_| format!("{option} has invalid value {value:?}"))
    }

    fn usage() -> String {
        "usage: calyx-gatectl [--socket PATH|--config PATH] [--profile NAME] [--response-fd FD] --json COMMAND ...".into()
    }
}
