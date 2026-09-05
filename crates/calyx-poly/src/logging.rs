//! Structured diagnostic logging for Poly operations.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use calyx_core::Clock;
use serde::{Deserialize, Serialize};

use crate::{PolyError, Result};

pub const POLY_STRUCTURED_LOG_SCHEMA_VERSION: &str = "poly.structured_log.v1";
pub const POLY_LOG_EVENT_RECORDED: &str = "CALYX_POLY_STRUCTURED_LOG_EVENT_RECORDED";
pub const POLY_LOG_MAX_CONTEXT_FIELDS: usize = 64;
pub const POLY_LOG_MAX_CONTEXT_VALUE_BYTES: usize = 4096;

/// Structured log level persisted in JSONL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolyLogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// One newline-delimited JSON event in the structured log source of truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolyLogEvent {
    pub schema_version: String,
    pub timestamp_unix_ms: u128,
    pub level: PolyLogLevel,
    pub component: String,
    pub action: String,
    pub code: String,
    pub message: String,
    pub error_kind: Option<String>,
    pub what_failed: Option<String>,
    pub how_to_fix: Option<String>,
    pub context: BTreeMap<String, String>,
}

impl PolyLogEvent {
    pub fn new(
        clock: &dyn Clock,
        level: PolyLogLevel,
        component: impl Into<String>,
        action: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
        context: BTreeMap<String, String>,
    ) -> Result<Self> {
        Ok(Self {
            schema_version: POLY_STRUCTURED_LOG_SCHEMA_VERSION.to_string(),
            timestamp_unix_ms: now_unix_ms(clock),
            level,
            component: component.into(),
            action: action.into(),
            code: code.into(),
            message: message.into(),
            error_kind: None,
            what_failed: None,
            how_to_fix: None,
            context,
        })
    }

    pub fn info(
        clock: &dyn Clock,
        component: impl Into<String>,
        action: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
        context: BTreeMap<String, String>,
    ) -> Result<Self> {
        Self::new(
            clock,
            PolyLogLevel::Info,
            component,
            action,
            code,
            message,
            context,
        )
    }

    pub fn error(
        clock: &dyn Clock,
        component: impl Into<String>,
        action: impl Into<String>,
        error: &PolyError,
        context: BTreeMap<String, String>,
    ) -> Result<Self> {
        let diagnostic = error.diagnostic();
        Ok(Self {
            schema_version: POLY_STRUCTURED_LOG_SCHEMA_VERSION.to_string(),
            timestamp_unix_ms: now_unix_ms(clock),
            level: PolyLogLevel::Error,
            component: component.into(),
            action: action.into(),
            code: diagnostic.code,
            message: diagnostic.message,
            error_kind: Some(diagnostic.kind),
            what_failed: Some(diagnostic.what_failed),
            how_to_fix: Some(diagnostic.how_to_fix),
            context,
        })
    }

    pub fn validate(&self) -> Result<()> {
        require_non_empty("schema_version", &self.schema_version)?;
        if self.schema_version != POLY_STRUCTURED_LOG_SCHEMA_VERSION {
            return Err(PolyError::structured_log(
                "POLY_LOG_SCHEMA_VERSION_INVALID",
                format!("unsupported log schema version {}", self.schema_version),
            ));
        }
        require_non_empty("component", &self.component)?;
        require_non_empty("action", &self.action)?;
        require_non_empty("code", &self.code)?;
        require_non_empty("message", &self.message)?;
        if !(self.code.starts_with("POLY_") || self.code.starts_with("CALYX_")) {
            return Err(PolyError::structured_log(
                "POLY_LOG_CODE_INVALID",
                format!("log code {} must start with POLY_ or CALYX_", self.code),
            ));
        }
        if self.context.len() > POLY_LOG_MAX_CONTEXT_FIELDS {
            return Err(PolyError::structured_log(
                "POLY_LOG_CONTEXT_TOO_LARGE",
                format!(
                    "context field count {} exceeds {}",
                    self.context.len(),
                    POLY_LOG_MAX_CONTEXT_FIELDS
                ),
            ));
        }
        for (key, value) in &self.context {
            require_non_empty("context key", key)?;
            if value.len() > POLY_LOG_MAX_CONTEXT_VALUE_BYTES {
                return Err(PolyError::structured_log(
                    "POLY_LOG_CONTEXT_VALUE_TOO_LARGE",
                    format!(
                        "context value for {key} is {} bytes, limit is {}",
                        value.len(),
                        POLY_LOG_MAX_CONTEXT_VALUE_BYTES
                    ),
                ));
            }
            if value.starts_with("sk-") && value.len() > 20 {
                return Err(PolyError::structured_log(
                    "POLY_LOG_SECRET_VALUE_REJECTED",
                    format!("context value for {key} looks like a secret value"),
                ));
            }
        }
        Ok(())
    }
}

/// JSONL sink that is the physical source of truth for structured diagnostics.
#[derive(Debug, Clone)]
pub struct StructuredLogSink {
    path: PathBuf,
}

impl StructuredLogSink {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(PolyError::structured_log(
                "POLY_LOG_PATH_EMPTY",
                "structured log path must not be empty",
            ));
        }
        if path.exists() && path.is_dir() {
            return Err(PolyError::structured_log(
                "POLY_LOG_PATH_IS_DIRECTORY",
                format!("structured log path {} is a directory", path.display()),
            ));
        }
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append_event(&self, event: &PolyLogEvent) -> Result<()> {
        event.validate()?;
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|err| {
                PolyError::structured_log(
                    "POLY_LOG_CREATE_DIR_FAILED",
                    format!("create log directory {}: {err}", parent.display()),
                )
            })?;
        }

        let line = serde_json::to_vec(event).map_err(|err| {
            PolyError::structured_log("POLY_LOG_ENCODE_FAILED", format!("encode log event: {err}"))
        })?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|err| {
                PolyError::structured_log(
                    "POLY_LOG_OPEN_FAILED",
                    format!("open structured log {}: {err}", self.path.display()),
                )
            })?;
        file.write_all(&line)
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|err| {
                PolyError::structured_log(
                    "POLY_LOG_WRITE_FAILED",
                    format!("write structured log {}: {err}", self.path.display()),
                )
            })?;
        file.sync_data().map_err(|err| {
            PolyError::structured_log(
                "POLY_LOG_SYNC_FAILED",
                format!("sync structured log {}: {err}", self.path.display()),
            )
        })?;
        emit_tracing_event(event);
        Ok(())
    }

    pub fn append_error(
        &self,
        clock: &dyn Clock,
        component: impl Into<String>,
        action: impl Into<String>,
        error: &PolyError,
        context: BTreeMap<String, String>,
    ) -> Result<()> {
        let event = PolyLogEvent::error(clock, component, action, error, context)?;
        self.append_event(&event)
    }

    pub fn read_events(&self) -> Result<Vec<PolyLogEvent>> {
        read_structured_log_events(&self.path)
    }
}

/// Reads and validates the physical JSONL log source of truth.
pub fn read_structured_log_events(path: &Path) -> Result<Vec<PolyLogEvent>> {
    let contents = fs::read_to_string(path).map_err(|err| {
        PolyError::structured_log(
            "POLY_LOG_READ_FAILED",
            format!("read structured log {}: {err}", path.display()),
        )
    })?;
    let mut events = Vec::new();
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(PolyError::structured_log(
                "POLY_LOG_EMPTY_LINE",
                format!(
                    "structured log {} line {} is empty",
                    path.display(),
                    index + 1
                ),
            ));
        }
        let event: PolyLogEvent = serde_json::from_str(line).map_err(|err| {
            PolyError::structured_log(
                "POLY_LOG_JSON_PARSE_FAILED",
                format!(
                    "parse structured log {} line {}: {err}",
                    path.display(),
                    index + 1
                ),
            )
        })?;
        event.validate()?;
        events.push(event);
    }
    Ok(events)
}

/// Extension trait for logging a failing `calyx-poly` result without swallowing it.
pub trait PolyResultLogExt<T> {
    fn log_error_context(
        self,
        clock: &dyn Clock,
        sink: &StructuredLogSink,
        component: &str,
        action: &str,
        context: BTreeMap<String, String>,
    ) -> Result<T>;
}

impl<T> PolyResultLogExt<T> for Result<T> {
    fn log_error_context(
        self,
        clock: &dyn Clock,
        sink: &StructuredLogSink,
        component: &str,
        action: &str,
        context: BTreeMap<String, String>,
    ) -> Result<T> {
        match self {
            Ok(value) => Ok(value),
            Err(error) => {
                sink.append_error(clock, component, action, &error, context)?;
                Err(error)
            }
        }
    }
}

pub fn log_context(pairs: &[(&str, String)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

fn emit_tracing_event(event: &PolyLogEvent) {
    let error_kind = event.error_kind.as_deref().unwrap_or("");
    let what_failed = event.what_failed.as_deref().unwrap_or("");
    let how_to_fix = event.how_to_fix.as_deref().unwrap_or("");
    match event.level {
        PolyLogLevel::Debug => tracing::debug!(
            target: "calyx_poly::structured_log",
            log_schema_version = %event.schema_version,
            log_component = %event.component,
            log_action = %event.action,
            log_code = %event.code,
            error_kind,
            what_failed,
            how_to_fix,
            "poly structured log event"
        ),
        PolyLogLevel::Info => tracing::info!(
            target: "calyx_poly::structured_log",
            log_schema_version = %event.schema_version,
            log_component = %event.component,
            log_action = %event.action,
            log_code = %event.code,
            "poly structured log event"
        ),
        PolyLogLevel::Warn => tracing::warn!(
            target: "calyx_poly::structured_log",
            log_schema_version = %event.schema_version,
            log_component = %event.component,
            log_action = %event.action,
            log_code = %event.code,
            "poly structured log event"
        ),
        PolyLogLevel::Error => tracing::error!(
            target: "calyx_poly::structured_log",
            log_schema_version = %event.schema_version,
            log_component = %event.component,
            log_action = %event.action,
            log_code = %event.code,
            error_kind,
            what_failed,
            how_to_fix,
            "poly structured log event"
        ),
    }
}

fn require_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(PolyError::structured_log(
            "POLY_LOG_FIELD_EMPTY",
            format!("{field} must not be empty"),
        ))
    } else {
        Ok(())
    }
}

fn now_unix_ms(clock: &dyn Clock) -> u128 {
    u128::from(clock.now())
}
