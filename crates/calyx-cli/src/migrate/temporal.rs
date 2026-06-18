use super::errors;
use crate::error::CliResult;

const MILLIS_THRESHOLD: u128 = 10_000_000_000;
const SECS_PER_DAY: i64 = 86_400;
const SECS_PER_HOUR: i64 = 3_600;
const SECS_PER_MINUTE: i64 = 60;

pub fn parse_event_time_secs(raw: &str, row_num: u64, field: &str) -> CliResult<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(timestamp_error(row_num, field, raw, "timestamp is empty").into());
    }
    if trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        return parse_unix_timestamp(trimmed, row_num, field, raw);
    }
    parse_utc_datetime(trimmed, row_num, field, raw)
}

fn parse_unix_timestamp(raw: &str, row_num: u64, field: &str, display: &str) -> CliResult<u64> {
    let value = raw.parse::<u128>().map_err(|err| {
        timestamp_error(row_num, field, display, format!("invalid integer: {err}"))
    })?;
    let seconds = if value >= MILLIS_THRESHOLD {
        value / 1_000
    } else {
        value
    };
    u64::try_from(seconds)
        .map_err(|_| timestamp_error(row_num, field, display, "timestamp exceeds u64").into())
        .and_then(|value| {
            i64::try_from(value).map(|_| value).map_err(|_| {
                timestamp_error(row_num, field, display, "timestamp exceeds i64").into()
            })
        })
}

fn parse_utc_datetime(raw: &str, row_num: u64, field: &str, display: &str) -> CliResult<u64> {
    let bytes = raw.as_bytes();
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return Err(timestamp_error(
            row_num,
            field,
            display,
            "expected Unix seconds/millis or YYYY-MM-DDTHH:MM:SSZ",
        )
        .into());
    }
    let parts = DateTimeParts {
        year: parse_i32_digits(&raw[0..4], row_num, field, display, "year")?,
        month: parse_u32_digits(&raw[5..7], row_num, field, display, "month")?,
        day: parse_u32_digits(&raw[8..10], row_num, field, display, "day")?,
        hour: parse_u32_digits(&raw[11..13], row_num, field, display, "hour")?,
        minute: parse_u32_digits(&raw[14..16], row_num, field, display, "minute")?,
        second: parse_u32_digits(&raw[17..19], row_num, field, display, "second")?,
    };
    validate_datetime(parts, row_num, field, display)?;
    let (tail, offset_secs) = parse_timezone_tail(&raw[19..], row_num, field, display)?;
    if !tail.is_empty() {
        return Err(timestamp_error(row_num, field, display, "unexpected timestamp suffix").into());
    }
    let seconds = days_from_civil(parts.year, parts.month, parts.day)
        .checked_mul(SECS_PER_DAY)
        .and_then(|value| value.checked_add(i64::from(parts.hour) * SECS_PER_HOUR))
        .and_then(|value| value.checked_add(i64::from(parts.minute) * SECS_PER_MINUTE))
        .and_then(|value| value.checked_add(i64::from(parts.second)))
        .and_then(|value| value.checked_sub(i64::from(offset_secs)))
        .ok_or_else(|| timestamp_error(row_num, field, display, "timestamp overflow"))?;
    u64::try_from(seconds).map_err(|_| {
        timestamp_error(
            row_num,
            field,
            display,
            "pre-1970 timestamps are unsupported",
        )
        .into()
    })
}

fn parse_timezone_tail<'a>(
    mut tail: &'a str,
    row_num: u64,
    field: &str,
    display: &str,
) -> CliResult<(&'a str, i32)> {
    if let Some(fraction) = tail.strip_prefix('.') {
        let digit_count = fraction
            .bytes()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digit_count == 0 {
            return Err(timestamp_error(row_num, field, display, "fraction has no digits").into());
        }
        tail = &fraction[digit_count..];
    }
    if tail.is_empty() || tail == "Z" {
        return Ok(("", 0));
    }
    let sign = match tail.as_bytes().first().copied() {
        Some(b'+') => 1,
        Some(b'-') => -1,
        _ => return Ok((tail, 0)),
    };
    if tail.len() != 6 || tail.as_bytes()[3] != b':' {
        return Err(timestamp_error(row_num, field, display, "timezone must be +/-HH:MM").into());
    }
    let hours = parse_u32_digits(&tail[1..3], row_num, field, display, "timezone hour")?;
    let minutes = parse_u32_digits(&tail[4..6], row_num, field, display, "timezone minute")?;
    if hours > 23 || minutes > 59 {
        return Err(
            timestamp_error(row_num, field, display, "timezone offset out of range").into(),
        );
    }
    Ok(("", sign * ((hours as i32 * 60 + minutes as i32) * 60)))
}

fn parse_i32_digits(
    value: &str,
    row_num: u64,
    field: &str,
    display: &str,
    part: &str,
) -> CliResult<i32> {
    value.parse::<i32>().map_err(|err| {
        timestamp_error(row_num, field, display, format!("{part} invalid: {err}")).into()
    })
}

fn parse_u32_digits(
    value: &str,
    row_num: u64,
    field: &str,
    display: &str,
    part: &str,
) -> CliResult<u32> {
    value.parse::<u32>().map_err(|err| {
        timestamp_error(row_num, field, display, format!("{part} invalid: {err}")).into()
    })
}

#[derive(Clone, Copy)]
struct DateTimeParts {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

fn validate_datetime(parts: DateTimeParts, row_num: u64, field: &str, display: &str) -> CliResult {
    let valid_date = (1..=12).contains(&parts.month)
        && (1..=days_in_month(parts.year, parts.month)).contains(&parts.day);
    let valid_time = parts.hour < 24 && parts.minute < 60 && parts.second < 60;
    if valid_date && valid_time {
        Ok(())
    } else {
        Err(timestamp_error(row_num, field, display, "date or time out of range").into())
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month_adj = i64::from(month) + if month > 2 { -3 } else { 9 };
    let doy = (153 * month_adj + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn timestamp_error(
    row_num: u64,
    field: &str,
    raw: &str,
    reason: impl Into<String>,
) -> calyx_core::CalyxError {
    errors::schema(format!(
        "row {row_num} {field} has invalid source event timestamp {raw:?}: {}",
        reason.into()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unix_seconds_millis_and_utc_text() {
        assert_eq!(
            parse_event_time_secs("1704204000", 1, "created_at").unwrap(),
            1_704_204_000
        );
        assert_eq!(
            parse_event_time_secs("1704204000123", 1, "created_at").unwrap(),
            1_704_204_000
        );
        assert_eq!(
            parse_event_time_secs("2024-01-02T14:00:00Z", 1, "created_at").unwrap(),
            1_704_204_000
        );
    }

    #[test]
    fn parses_sqlite_text_fraction_and_offsets() {
        assert_eq!(
            parse_event_time_secs("2024-01-02 14:00:00.999", 1, "created_at").unwrap(),
            1_704_204_000
        );
        assert_eq!(
            parse_event_time_secs("2024-01-02T14:00:00+02:00", 1, "created_at").unwrap(),
            1_704_196_800
        );
    }

    #[test]
    fn malformed_timestamps_fail_closed() {
        for raw in ["now", "", "2024-02-30T00:00:00Z", "2024-01-02T25:00:00Z"] {
            assert_eq!(
                parse_event_time_secs(raw, 7, "created_at")
                    .unwrap_err()
                    .code(),
                errors::CALYX_MIGRATE_SQLITE_SCHEMA
            );
        }
    }
}
