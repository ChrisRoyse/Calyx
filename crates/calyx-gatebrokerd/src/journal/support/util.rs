use std::time::{SystemTime, UNIX_EPOCH};

use super::super::JournalError;

pub(in crate::journal) fn now_ms() -> Result<i64, JournalError> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| JournalError::Durability(format!("system clock precedes epoch: {error}")))?
        .as_millis();
    i64::try_from(value)
        .map_err(|_| JournalError::Durability("timestamp exceeds Turso integer range".into()))
}

pub(in crate::journal) fn parse_u64(value: &str, field: &str) -> Result<u64, JournalError> {
    value
        .parse()
        .map_err(|error| JournalError::Corrupt(format!("invalid {field} {value:?}: {error}")))
}

pub(in crate::journal) fn validate_optional_text(
    field: &str,
    value: Option<&str>,
    maximum: usize,
) -> Result<(), JournalError> {
    if value.is_some_and(|value| {
        value.is_empty() || value.len() > maximum || value.contains(['\0', '\r', '\n'])
    }) {
        return Err(JournalError::InvalidMetadata(format!(
            "{field} is empty, oversized, or contains control bytes"
        )));
    }
    Ok(())
}

pub(in crate::journal) fn sql(operation: &'static str, source: turso::Error) -> JournalError {
    JournalError::Sql { operation, source }
}
pub(in crate::journal) fn corrupt_protocol(error: crate::protocol::ProtocolError) -> JournalError {
    JournalError::Corrupt(error.to_string())
}
pub(in crate::journal) fn corrupt_sql(error: turso::Error) -> JournalError {
    JournalError::Corrupt(error.to_string())
}
