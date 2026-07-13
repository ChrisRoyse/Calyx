use std::collections::BTreeMap;

use crate::logging::{self, Level};
use crate::protocol::{
    ContextKey, ContextValue, ErrorResponse, ErrorText, PROTOCOL_VERSION, RequestId,
    ResponseEnvelope, ResponseOutcome, StableCode,
};

#[derive(Debug, Clone)]
pub struct BrokerError {
    pub code: StableCode,
    pub message: String,
    pub remediation: String,
    pub context: BTreeMap<String, String>,
    pub fatal: bool,
}

impl BrokerError {
    pub fn new(
        code: StableCode,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            remediation: remediation.into(),
            context: BTreeMap::new(),
            fatal: false,
        }
    }

    pub fn context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }

    pub fn fatal(mut self) -> Self {
        self.fatal = true;
        self
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(
            StableCode::InvalidRequest,
            message,
            "Correct the bounded request fields and retry with a new request id.",
        )
    }

    pub fn permission(message: impl Into<String>) -> Self {
        Self::new(
            StableCode::PermissionDenied,
            message,
            "Use a live descendant of the bound controller in the configured client group.",
        )
    }

    pub fn journal(operation: &str, error: impl std::fmt::Display) -> Self {
        Self::new(
            StableCode::JournalFailure,
            format!("durable journal failed during {operation}: {error}"),
            "Inspect the SQLite file, WAL, filesystem capacity, ownership, and broker journal logs before retrying.",
        )
        .fatal()
    }

    pub fn system(operation: &str, error: impl std::fmt::Display) -> Self {
        Self::new(
            StableCode::SystemFailure,
            format!("system operation failed during {operation}: {error}"),
            "Inspect the named syscall/manager operation and the structured broker journal entry.",
        )
    }

    pub fn response(&self, request_id: RequestId) -> ResponseEnvelope {
        let message = ErrorText::new(bounded(&self.message, 2_048, "broker request failed"))
            .expect("bounded nonempty error message");
        let remediation = ErrorText::new(bounded(
            &self.remediation,
            2_048,
            "Inspect the broker logs and source-of-truth state.",
        ))
        .expect("bounded nonempty remediation");
        let context = self
            .context
            .iter()
            .filter_map(|(key, value)| {
                let key = ContextKey::new(bounded(key, 96, "context")).ok()?;
                let value = ContextValue::new(bounded(value, 1_024, "unavailable")).ok()?;
                Some((key, value))
            })
            .collect();
        ResponseEnvelope {
            version: PROTOCOL_VERSION,
            request_id,
            outcome: ResponseOutcome::Error(ErrorResponse {
                code: self.code,
                message,
                remediation,
                context,
            }),
        }
    }

    pub fn log(&self, event: &str) {
        logging::emit(
            if self.fatal {
                Level::Critical
            } else {
                Level::Error
            },
            event,
            code_name(self.code),
            &self.message,
            &self.remediation,
            &self.context,
        );
    }
}

fn bounded(value: &str, maximum: usize, fallback: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|value| {
            if matches!(value, '\0' | '\r' | '\n') {
                ' '
            } else {
                value
            }
        })
        .collect();
    let sanitized = sanitized.trim();
    if sanitized.is_empty() {
        return fallback.into();
    }
    if sanitized.len() <= maximum {
        return sanitized.into();
    }
    let mut end = maximum;
    while !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    sanitized[..end].into()
}

pub fn code_name(code: StableCode) -> &'static str {
    match code {
        StableCode::InvalidFrame => "CALYX_GATEBROKER_INVALID_FRAME",
        StableCode::ProtocolVersionMismatch => "CALYX_GATEBROKER_PROTOCOL_VERSION_MISMATCH",
        StableCode::InvalidRequest => "CALYX_GATEBROKER_INVALID_REQUEST",
        StableCode::ConfigInvalid => "CALYX_GATEBROKER_CONFIG_INVALID",
        StableCode::JournalFailure => "CALYX_GATEBROKER_JOURNAL_FAILURE",
        StableCode::InvalidTransition => "CALYX_GATEBROKER_INVALID_TRANSITION",
        StableCode::NotFound => "CALYX_GATEBROKER_NOT_FOUND",
        StableCode::Busy => "CALYX_GATEBROKER_BUSY",
        StableCode::OwnerDied => "CALYX_GATEBROKER_OWNER_DIED",
        StableCode::RecoveryRequired => "CALYX_GATEBROKER_RECOVERY_REQUIRED",
        StableCode::ObjectCollision => "CALYX_GATEBROKER_OBJECT_COLLISION",
        StableCode::ObjectMismatch => "CALYX_GATEBROKER_OBJECT_MISMATCH",
        StableCode::CapabilityUnavailable => "CALYX_GATEBROKER_CAPABILITY_UNAVAILABLE",
        StableCode::PermissionDenied => "CALYX_GATEBROKER_PERMISSION_DENIED",
        StableCode::StageFailed => "CALYX_GATEBROKER_STAGE_FAILED",
        StableCode::SystemFailure => "CALYX_GATEBROKER_SYSTEM_FAILURE",
        StableCode::Internal => "CALYX_GATEBROKER_INTERNAL",
    }
}
