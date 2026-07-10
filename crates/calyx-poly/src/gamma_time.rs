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

pub(crate) fn iso8601_to_unix(s: &str) -> Option<u64> {
    let bytes = s.as_bytes();
    if bytes.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-' {
        return iso8601_to_unix(&format!("{s}T00:00:00Z"));
    }
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || (bytes[10] != b'T' && bytes[10] != b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let p = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (p(0, 4)?, p(5, 7)?, p(8, 10)?);
    let (h, mi, se) = (p(11, 13)?, p(14, 16)?, p(17, 19)?);
    if !(1..=12).contains(&mo)
        || !(1..=days_in_month(y, mo)?).contains(&d)
        || !(0..=23).contains(&h)
        || !(0..=59).contains(&mi)
        || !(0..=59).contains(&se)
    {
        return None;
    }
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let local = days
        .checked_mul(86_400)?
        .checked_add(h * 3_600 + mi * 60 + se)?;
    let utc = local.checked_sub(utc_offset_seconds(&s[19..])?)?;
    u64::try_from(utc).ok()
}

fn utc_offset_seconds(suffix: &str) -> Option<i64> {
    let zone = if let Some(fraction_and_zone) = suffix.strip_prefix('.') {
        let zone_start = fraction_and_zone
            .find(['Z', '+', '-'])
            .unwrap_or(fraction_and_zone.len());
        let (fraction, zone) = fraction_and_zone.split_at(zone_start);
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        zone
    } else {
        suffix
    };
    if zone.is_empty() || zone == "Z" {
        return Some(0);
    }
    let bytes = zone.as_bytes();
    if bytes.len() != 6 || bytes[3] != b':' || !matches!(bytes[0], b'+' | b'-') {
        return None;
    }
    let hours = zone.get(1..3)?.parse::<i64>().ok()?;
    let minutes = zone.get(4..6)?.parse::<i64>().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    let seconds = hours * 3_600 + minutes * 60;
    Some(if bytes[0] == b'+' { seconds } else { -seconds })
}

fn days_in_month(year: i64, month: i64) -> Option<i64> {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => Some(31),
        4 | 6 | 9 | 11 => Some(30),
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => Some(29),
        2 => Some(28),
        _ => None,
    }
}

fn invalid(message: impl Into<String>) -> PolyError {
    PolyError::raw_source(ERR_GAMMA_MARKET_INVALID, message.into())
}

#[cfg(test)]
mod tests {
    use super::iso8601_to_unix;

    #[test]
    fn issue1394_iso8601_normalizes_offsets_and_rejects_trailing_data() {
        let utc = iso8601_to_unix("2026-01-02T03:04:05Z").unwrap();
        assert_eq!(iso8601_to_unix("2026-01-02T05:34:05+02:30"), Some(utc));
        assert_eq!(iso8601_to_unix("2026-01-01T22:04:05-05:00"), Some(utc));
        assert_eq!(iso8601_to_unix("2026-01-02T03:04:05.987Z"), Some(utc));
        assert_eq!(iso8601_to_unix("2026-01-02T03:04:05Zjunk"), None);
        assert_eq!(iso8601_to_unix("2026-02-29T03:04:05Z"), None);
        assert_eq!(iso8601_to_unix("2026-01-02T03:04:05+24:00"), None);
    }
}
