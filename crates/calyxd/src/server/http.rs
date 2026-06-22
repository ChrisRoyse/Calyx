use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub const REQUEST_HEAD_LIMIT: usize = 8192;
pub const DEFAULT_BODY_LIMIT: usize = 1024 * 1024;
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

pub struct HttpResponse {
    pub status: &'static str,
    pub content_type: &'static str,
    pub body: String,
}

#[derive(Debug)]
pub enum HttpReadError {
    BadRequest(String),
    BodyTooLarge { limit: usize, actual: usize },
}

impl HttpReadError {
    pub fn status(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "400 Bad Request",
            Self::BodyTooLarge { .. } => "413 Payload Too Large",
        }
    }

    pub fn body(&self) -> String {
        match self {
            Self::BadRequest(detail) => {
                let _ = detail.len();
                "bad request\n".to_string()
            }
            Self::BodyTooLarge { limit, actual } => {
                format!("request body {actual} bytes exceeds {limit}\n")
            }
        }
    }
}

pub fn read_request(
    stream: &mut TcpStream,
    max_body_bytes: usize,
) -> Result<HttpRequest, HttpReadError> {
    let head = read_head(stream)?;
    let mut lines = head.lines();
    let request_line = lines
        .next()
        .ok_or_else(|| HttpReadError::BadRequest("empty request head".to_string()))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    if method.is_empty() || path.is_empty() {
        return Err(HttpReadError::BadRequest(
            "request line missing method or path".to_string(),
        ));
    }

    let mut headers = Vec::new();
    for line in lines {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(HttpReadError::BadRequest(format!(
                "malformed header `{line}`"
            )));
        };
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }
    let content_length = content_length(&headers)?;
    if content_length > max_body_bytes {
        return Err(HttpReadError::BodyTooLarge {
            limit: max_body_bytes,
            actual: content_length,
        });
    }
    let mut body = vec![0_u8; content_length];
    if content_length > 0 {
        stream
            .read_exact(&mut body)
            .map_err(|error| HttpReadError::BadRequest(format!("read body: {error}")))?;
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

pub fn write_response(stream: &mut TcpStream, response: &HttpResponse) -> Result<(), String> {
    let wire = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.status,
        response.content_type,
        response.body.len(),
        response.body
    );
    stream
        .write_all(wire.as_bytes())
        .map_err(|error| format!("write response: {error}"))
}

fn read_head(stream: &mut TcpStream) -> Result<String, HttpReadError> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while head.len() < REQUEST_HEAD_LIMIT {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") || head.ends_with(b"\n\n") {
                    break;
                }
            }
            Err(error) => return Err(HttpReadError::BadRequest(format!("read head: {error}"))),
        }
    }
    if head.len() >= REQUEST_HEAD_LIMIT {
        return Err(HttpReadError::BadRequest(format!(
            "request head exceeds {REQUEST_HEAD_LIMIT} bytes"
        )));
    }
    String::from_utf8(head)
        .map_err(|error| HttpReadError::BadRequest(format!("request head is not utf-8: {error}")))
}

fn content_length(headers: &[(String, String)]) -> Result<usize, HttpReadError> {
    let Some((_, value)) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
    else {
        return Ok(0);
    };
    value
        .parse::<usize>()
        .map_err(|_| HttpReadError::BadRequest("invalid content-length".to_string()))
}
