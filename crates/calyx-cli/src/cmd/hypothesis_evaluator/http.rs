use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use calyx_core::CalyxError;
use serde_json::Value;

use super::{evaluator_endpoint_error, evaluator_malformed};
use crate::error::{CliError, CliResult};

pub(super) struct EvaluatorAuth {
    bearer_token: Option<String>,
}

impl EvaluatorAuth {
    pub(super) fn for_endpoint(endpoint: &str, auth_env: Option<&str>) -> CliResult<Self> {
        Self::for_endpoint_with_lookup(endpoint, auth_env, |name| std::env::var(name).ok())
    }

    fn for_endpoint_with_lookup<F>(
        endpoint: &str,
        auth_env: Option<&str>,
        lookup: F,
    ) -> CliResult<Self>
    where
        F: FnOnce(&str) -> Option<String>,
    {
        if !endpoint.starts_with("https://") {
            return Ok(Self { bearer_token: None });
        }
        let env = auth_env
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| auth_missing("https evaluator endpoints require --auth-env <VAR>"))?;
        let token = lookup(env)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| auth_missing(format!("{env} is unset or empty")))?;
        Ok(Self {
            bearer_token: Some(token),
        })
    }

    fn authorization_header(&self) -> CliResult<String> {
        let token = self.bearer_token.as_deref().ok_or_else(|| {
            auth_missing("https evaluator endpoint was invoked without bearer-token auth")
        })?;
        Ok(format!("Bearer {token}"))
    }
}

pub(super) fn artifact_endpoint(endpoint: &str) -> String {
    sanitize_endpoint(endpoint)
}

pub(super) fn post_json(
    endpoint: &str,
    body: &Value,
    timeout: Duration,
    auth: &EvaluatorAuth,
) -> CliResult<Value> {
    if endpoint.starts_with("https://") {
        return post_https_json(endpoint, body, timeout, auth, send_ureq_https);
    }
    post_plain_http_json(endpoint, body, timeout)
}

fn post_plain_http_json(endpoint: &str, body: &Value, timeout: Duration) -> CliResult<Value> {
    let parsed = parse_http_url(endpoint)?;
    let safe_endpoint = sanitize_endpoint(endpoint);
    let addr = (parsed.host.as_str(), parsed.port)
        .to_socket_addrs()
        .map_err(|err| evaluator_endpoint_error(format!("resolve {safe_endpoint}: {err}")))?
        .next()
        .ok_or_else(|| {
            evaluator_endpoint_error(format!("resolve {safe_endpoint}: no addresses"))
        })?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|err| evaluator_endpoint_error(format!("connect {safe_endpoint}: {err}")))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|err| evaluator_endpoint_error(format!("set read timeout: {err}")))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|err| evaluator_endpoint_error(format!("set write timeout: {err}")))?;
    let payload = serde_json::to_vec(body)
        .map_err(|err| evaluator_malformed(format!("serialize evaluator request: {err}")))?;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        parsed.path,
        parsed.host_header,
        payload.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| evaluator_endpoint_error(format!("write request: {err}")))?;
    stream
        .write_all(&payload)
        .map_err(|err| evaluator_endpoint_error(format!("write request body: {err}")))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|err| evaluator_endpoint_error(format!("read response: {err}")))?;
    parse_http_response(&response)
}

fn post_https_json(
    endpoint: &str,
    body: &Value,
    timeout: Duration,
    auth: &EvaluatorAuth,
    sender: impl FnOnce(HttpsRequest) -> CliResult<Value>,
) -> CliResult<Value> {
    sender(HttpsRequest {
        endpoint: endpoint.to_string(),
        safe_endpoint: sanitize_endpoint(endpoint),
        authorization: auth.authorization_header()?,
        timeout,
        body: body.clone(),
    })
}

struct HttpsRequest {
    endpoint: String,
    safe_endpoint: String,
    authorization: String,
    timeout: Duration,
    body: Value,
}

fn send_ureq_https(request: HttpsRequest) -> CliResult<Value> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(request.timeout))
        .build()
        .into();
    let mut response = agent
        .post(&request.endpoint)
        .header("Authorization", &request.authorization)
        .header("Accept", "application/json")
        .send_json(&request.body)
        .map_err(|err| map_https_error(err, &request.safe_endpoint))?;
    response
        .body_mut()
        .read_json::<Value>()
        .map_err(|err| evaluator_malformed(format!("parse HTTPS JSON response: {err}")).into())
}

struct ParsedUrl {
    host: String,
    host_header: String,
    port: u16,
    path: String,
}

fn parse_http_url(endpoint: &str) -> CliResult<ParsedUrl> {
    let rest = endpoint.strip_prefix("http://").ok_or_else(|| {
        evaluator_endpoint_error("only http:// or https:// evaluator endpoints are supported")
    })?;
    let (authority, path) = match rest.split_once('/') {
        Some((host, path)) => (host, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = authority.rsplit_once(':').ok_or_else(|| {
        evaluator_endpoint_error("evaluator endpoint must include explicit host:port")
    })?;
    if host.trim().is_empty() {
        return Err(evaluator_endpoint_error("evaluator endpoint host is empty").into());
    }
    let port = port
        .parse::<u16>()
        .map_err(|err| evaluator_endpoint_error(format!("parse evaluator endpoint port: {err}")))?;
    Ok(ParsedUrl {
        host: host.to_string(),
        host_header: authority.to_string(),
        port,
        path,
    })
}

fn parse_http_response(response: &[u8]) -> CliResult<Value> {
    let marker = b"\r\n\r\n";
    let split = response
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or_else(|| evaluator_malformed("endpoint response missing HTTP header terminator"))?;
    let headers = String::from_utf8_lossy(&response[..split]);
    let status = headers
        .lines()
        .next()
        .ok_or_else(|| evaluator_malformed("endpoint response missing status line"))?;
    if !status.contains(" 200 ") {
        return Err(
            evaluator_endpoint_error(format!("endpoint returned non-200 status {status}")).into(),
        );
    }
    let body = &response[split + marker.len()..];
    serde_json::from_slice(body)
        .map_err(|err| evaluator_malformed(format!("parse endpoint JSON response: {err}")).into())
}

fn map_https_error(error: ureq::Error, endpoint: &str) -> CliError {
    match error {
        ureq::Error::StatusCode(401 | 403) => auth_failed(format!(
            "HTTPS evaluator endpoint {endpoint} rejected bearer-token auth"
        )),
        ureq::Error::StatusCode(code) => CliError::Calyx(CalyxError {
            code: "CALYX_HYPOTHESIS_EVALUATOR_ENDPOINT_STATUS",
            message: format!("HTTPS evaluator endpoint {endpoint} returned status {code}"),
            remediation: "restore a healthy evaluator endpoint or choose a supported model",
        }),
        other => evaluator_endpoint_error(format!("HTTPS endpoint {endpoint}: {other}")).into(),
    }
}

fn sanitize_endpoint(endpoint: &str) -> String {
    let Some((scheme, rest)) = endpoint.split_once("://") else {
        return endpoint.to_string();
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    let clean_path = path.split(['?', '#']).next().unwrap_or("");
    if clean_path.is_empty() {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}/{clean_path}")
    }
}

fn auth_missing(message: impl Into<String>) -> CliError {
    CliError::Calyx(CalyxError {
        code: "CALYX_HYPOTHESIS_EVALUATOR_AUTH_MISSING",
        message: message.into(),
        remediation: "set --auth-env to the name of an environment variable containing the bearer token",
    })
}

fn auth_failed(message: impl Into<String>) -> CliError {
    CliError::Calyx(CalyxError {
        code: "CALYX_HYPOTHESIS_EVALUATOR_AUTH_FAILED",
        message: message.into(),
        remediation: "verify the evaluator bearer token, endpoint, and model authorization",
    })
}

#[cfg(test)]
pub(crate) fn auth_for_endpoint_with_lookup_for_test<F>(
    endpoint: &str,
    auth_env: Option<&str>,
    lookup: F,
) -> CliResult<EvaluatorAuth>
where
    F: FnOnce(&str) -> Option<String>,
{
    EvaluatorAuth::for_endpoint_with_lookup(endpoint, auth_env, lookup)
}

#[cfg(test)]
pub(crate) fn post_https_json_with_sender_for_test(
    endpoint: &str,
    body: &Value,
    timeout: Duration,
    auth: &EvaluatorAuth,
    sender: impl FnOnce(String, String, Duration, Value) -> CliResult<Value>,
) -> CliResult<Value> {
    post_https_json(endpoint, body, timeout, auth, |request| {
        sender(
            request.safe_endpoint,
            request.authorization,
            request.timeout,
            request.body,
        )
    })
}
