use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::str;
use std::time::Duration;

use calyx_core::CalyxError;
use serde::{Deserialize, Serialize};

use super::artifact::{Artifact, artifact};
use super::log::write_json_file;
use crate::error::{CliError, CliResult};

const INFO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_INFO_RESPONSE_BYTES: usize = 64 * 1024;

pub(super) struct CommissionedTei {
    pub(super) artifacts: Vec<Artifact>,
    pub(super) source_hf_id: String,
    pub(super) requested_hf_id: Option<String>,
    pub(super) descriptor_path: PathBuf,
}

#[derive(Deserialize)]
struct TeiInfo {
    model_id: Option<String>,
    served_model_name: Option<String>,
    model_sha: Option<String>,
    model_dtype: Option<String>,
}

#[derive(Serialize)]
struct TeiDescriptor {
    source_hf_id: String,
    served_model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_hf_id: Option<String>,
    endpoint: String,
    info_endpoint: String,
    modality: String,
    dim: u32,
    norm: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_dtype: Option<String>,
}

pub(super) fn write_descriptor(
    requested_hf_id: &str,
    endpoint: String,
    dim: u32,
    out: &Path,
) -> CliResult<CommissionedTei> {
    let info_endpoint = HttpEndpoint::parse(&endpoint)?.info_url();
    let info = read_info(&info_endpoint)?;
    let source_hf_id = served_model_id(&info)?;
    let requested_hf_id = mismatch_request(requested_hf_id, &source_hf_id);
    let descriptor = TeiDescriptor {
        source_hf_id: source_hf_id.clone(),
        served_model_id: source_hf_id.clone(),
        requested_hf_id: requested_hf_id.clone(),
        endpoint,
        info_endpoint,
        modality: "text".to_string(),
        dim,
        norm: "unit".to_string(),
        model_sha: info.model_sha,
        model_dtype: info.model_dtype,
    };
    let path = out.join("tei-descriptor.json");
    write_json_file(&path, &descriptor)?;
    Ok(CommissionedTei {
        artifacts: vec![artifact("model", path.clone())?],
        source_hf_id,
        requested_hf_id,
        descriptor_path: path,
    })
}

fn read_info(info_endpoint: &str) -> CliResult<TeiInfo> {
    let body = get_json(info_endpoint, INFO_TIMEOUT)?;
    serde_json::from_slice(&body).map_err(|err| {
        CliError::from(CalyxError::lens_unreachable(format!(
            "parse TEI /info failed: {err}"
        )))
    })
}

fn served_model_id(info: &TeiInfo) -> CliResult<String> {
    for value in [&info.model_id, &info.served_model_name] {
        if let Some(model_id) = value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(model_id.to_string());
        }
    }
    Err(CliError::from(CalyxError::lens_unreachable(
        "TEI /info did not expose model_id or served_model_name",
    )))
}

fn mismatch_request(requested: &str, served: &str) -> Option<String> {
    let requested = requested.trim();
    if requested.eq_ignore_ascii_case(served) {
        None
    } else {
        Some(requested.to_string())
    }
}

fn get_json(endpoint: &str, timeout: Duration) -> CliResult<Vec<u8>> {
    let endpoint = HttpEndpoint::parse(endpoint)?;
    let address = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|err| {
            CliError::from(CalyxError::lens_unreachable(format!(
                "resolve TEI /info endpoint failed: {err}"
            )))
        })?
        .next()
        .ok_or_else(|| {
            CliError::from(CalyxError::lens_unreachable(
                "TEI /info endpoint resolved no addresses",
            ))
        })?;
    let mut stream = TcpStream::connect_timeout(&address, timeout).map_err(|err| {
        CliError::from(CalyxError::lens_unreachable(format!(
            "connect TEI /info endpoint failed: {err}"
        )))
    })?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        endpoint.path,
        endpoint.authority()
    );
    stream.write_all(request.as_bytes())?;
    let mut response = Vec::new();
    stream
        .take((MAX_INFO_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut response)?;
    if response.len() > MAX_INFO_RESPONSE_BYTES {
        return Err(CliError::from(CalyxError::lens_unreachable(
            "TEI /info response exceeded 64KiB",
        )));
    }
    parse_http_response(&response)
}

fn parse_http_response(response: &[u8]) -> CliResult<Vec<u8>> {
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            CliError::from(CalyxError::lens_unreachable(
                "TEI /info response missing header terminator",
            ))
        })?;
    let headers = str::from_utf8(&response[..split]).map_err(|err| {
        CliError::from(CalyxError::lens_unreachable(format!(
            "TEI /info headers invalid UTF-8: {err}"
        )))
    })?;
    let status = headers.lines().next().unwrap_or_default();
    if !status.contains(" 200 ") {
        let preview = String::from_utf8_lossy(&response[split + 4..]);
        return Err(CliError::from(CalyxError::lens_unreachable(format!(
            "TEI /info HTTP status {status}: {}",
            preview.chars().take(120).collect::<String>()
        ))));
    }
    let body = &response[split + 4..];
    if headers
        .lines()
        .any(|line| line.eq_ignore_ascii_case("transfer-encoding: chunked"))
    {
        decode_chunked(body)
    } else {
        Ok(body.to_vec())
    }
}

fn decode_chunked(mut body: &[u8]) -> CliResult<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = body
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| {
                CliError::from(CalyxError::lens_unreachable(
                    "TEI /info chunk size missing CRLF",
                ))
            })?;
        let size_text = str::from_utf8(&body[..line_end]).map_err(|err| {
            CliError::from(CalyxError::lens_unreachable(format!(
                "TEI /info chunk size UTF-8: {err}"
            )))
        })?;
        let size_hex = size_text.split(';').next().unwrap_or_default();
        let size = usize::from_str_radix(size_hex.trim(), 16).map_err(|err| {
            CliError::from(CalyxError::lens_unreachable(format!(
                "TEI /info chunk size parse failed: {err}"
            )))
        })?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size + 2 {
            return Err(CliError::from(CalyxError::lens_unreachable(
                "TEI /info chunk body truncated",
            )));
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

struct HttpEndpoint {
    host: String,
    port: u16,
    path: String,
}

impl HttpEndpoint {
    fn parse(endpoint: &str) -> CliResult<Self> {
        let rest = endpoint.strip_prefix("http://").ok_or_else(|| {
            CliError::from(CalyxError::lens_unreachable(
                "TEI endpoint must use http://",
            ))
        })?;
        let (authority, path) = rest
            .split_once('/')
            .map(|(authority, path)| (authority, format!("/{path}")))
            .unwrap_or((rest, "/".to_string()));
        let (host, port) = if let Some((host, port)) = authority.rsplit_once(':') {
            let parsed = port.parse::<u16>().map_err(|err| {
                CliError::from(CalyxError::lens_unreachable(format!(
                    "TEI port parse failed: {err}"
                )))
            })?;
            (host.to_string(), parsed)
        } else {
            (authority.to_string(), 80)
        };
        if host.is_empty() {
            return Err(CliError::from(CalyxError::lens_unreachable(
                "TEI endpoint host is empty",
            )));
        }
        Ok(Self { host, port, path })
    }

    fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    fn info_url(&self) -> String {
        format!("http://{}/info", self.authority())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    #[test]
    fn endpoint_with_embed_path_maps_to_info_endpoint() {
        let endpoint = HttpEndpoint::parse("http://127.0.0.1:8088/embed").unwrap();

        assert_eq!(endpoint.info_url(), "http://127.0.0.1:8088/info");
    }

    #[test]
    fn served_model_id_prefers_model_id() {
        let info = TeiInfo {
            model_id: Some("Alibaba-NLP/gte-multilingual-base".to_string()),
            served_model_name: Some("alias".to_string()),
            model_sha: None,
            model_dtype: None,
        };

        assert_eq!(
            served_model_id(&info).unwrap(),
            "Alibaba-NLP/gte-multilingual-base"
        );
    }

    #[test]
    fn missing_served_model_id_fails_closed() {
        let info = TeiInfo {
            model_id: Some(" ".to_string()),
            served_model_name: None,
            model_sha: None,
            model_dtype: None,
        };

        let error = served_model_id(&info).unwrap_err();

        assert_eq!(error.code(), "CALYX_LENS_UNREACHABLE");
    }

    #[test]
    fn requested_id_is_only_recorded_on_mismatch() {
        assert_eq!(
            mismatch_request("thenlper/gte-base", "Alibaba-NLP/gte-multilingual-base"),
            Some("thenlper/gte-base".to_string())
        );
        assert_eq!(
            mismatch_request(
                "Alibaba-NLP/gte-multilingual-base",
                "Alibaba-NLP/gte-multilingual-base"
            ),
            None
        );
    }

    #[test]
    fn info_read_uses_endpoint_host_port_and_info_path() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 512];
            let size = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..size]).to_string();
            let body = br#"{"model_id":"served/model","model_sha":"abc","model_dtype":"float16"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                str::from_utf8(body).unwrap()
            );
            stream.write_all(response.as_bytes()).unwrap();
            request
        });
        let endpoint = format!("http://{addr}/embed");

        let info = read_info(&HttpEndpoint::parse(&endpoint).unwrap().info_url()).unwrap();
        let request = worker.join().unwrap();

        assert!(request.starts_with("GET /info HTTP/1.1"));
        assert_eq!(served_model_id(&info).unwrap(), "served/model");
        assert_eq!(info.model_sha.as_deref(), Some("abc"));
    }
}
