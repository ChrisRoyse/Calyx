use serde_json::Value;

use crate::error::{PolyError, Result};
use crate::gamma_client::ERR_GAMMA_MARKET_INVALID;

pub(crate) fn first_timestamp(value: &Value, fields: &[&str]) -> Result<Option<u64>> {
    for field in fields {
        if value.get(*field).is_some() {
            return optional_timestamp(value, field);
        }
    }
    Ok(None)
}

fn optional_timestamp(value: &Value, field: &str) -> Result<Option<u64>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => Ok(Some(normalize_unix_ts(number.as_u64().ok_or_else(
            || {
                invalid(format!(
                    "Gamma timestamp field {field} must be an unsigned integer"
                ))
            },
        )?))),
        Some(Value::String(text)) => parse_timestamp_string(text, field),
        Some(other) => Err(invalid(format!(
            "Gamma timestamp field {field} expected string/integer, got {other}"
        ))),
    }
}

fn parse_timestamp_string(text: &str, field: &str) -> Result<Option<u64>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if let Ok(raw) = trimmed.parse::<u64>() {
        return Ok(Some(normalize_unix_ts(raw)));
    }
    Ok(Some(iso8601_to_unix(trimmed).ok_or_else(|| {
        invalid(format!(
            "Gamma timestamp field {field} is not parseable as UTC ISO-8601"
        ))
    })?))
}

fn normalize_unix_ts(value: u64) -> u64 {
    if value > 10_000_000_000 {
        value / 1000
    } else {
        value
    }
}

fn iso8601_to_unix(s: &str) -> Option<u64> {
    let bytes = s.as_bytes();
    if bytes.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-' {
        return iso8601_to_unix(&format!("{s}T00:00:00Z"));
    }
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || (bytes[10] != b'T' && bytes[10] != b' ')
    {
        return None;
    }
    let p = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (p(0, 4)?, p(5, 7)?, p(8, 10)?);
    let (h, mi, se) = (p(11, 13)?, p(14, 16)?, p(17, 19)?);
    if !(1..=12).contains(&mo)
        || !(1..=31).contains(&d)
        || !(0..=23).contains(&h)
        || !(0..=59).contains(&mi)
        || !(0..=60).contains(&se)
    {
        return None;
    }
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3_600 + mi * 60 + se).ok()
}

fn invalid(message: impl Into<String>) -> PolyError {
    PolyError::raw_source(ERR_GAMMA_MARKET_INVALID, message.into())
}
