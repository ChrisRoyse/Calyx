//! One-record-per-line structured diagnostics suitable for journald.

use std::collections::BTreeMap;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Serialize)]
struct Record<'a> {
    schema: &'static str,
    timestamp_ms: u128,
    pid: u32,
    level: Level,
    event: &'a str,
    code: &'a str,
    message: &'a str,
    remediation: &'a str,
    context: &'a BTreeMap<String, String>,
}

/// Emit a bounded structured diagnostic. Callers must never place run tokens,
/// environment values, or complete argv values in `context`.
pub fn emit(
    level: Level,
    event: &str,
    code: &str,
    message: &str,
    remediation: &str,
    context: &BTreeMap<String, String>,
) {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default();
    let record = Record {
        schema: "calyx-gatebroker-log-v1",
        timestamp_ms,
        pid: std::process::id(),
        level,
        event,
        code,
        message,
        remediation,
        context,
    };
    let mut stderr = std::io::stderr().lock();
    match serde_json::to_writer(&mut stderr, &record) {
        Ok(()) => {
            let _ = stderr.write_all(b"\n");
        }
        Err(error) => {
            let _ = writeln!(
                stderr,
                "calyx-gatebrokerd: CRITICAL diagnostic serialization failed: {error}"
            );
        }
    }
}
