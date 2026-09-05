use calyx_core::Result;

use super::error::{CALYX_STRUCTURED_DATE_INVALID, structured_runtime_error};

#[derive(Clone, Copy, Debug)]
pub(super) struct GregorianDate {
    pub(super) year: u32,
    pub(super) month: u32,
    pub(super) day: u32,
}

pub(super) fn parse_gregorian_date(value: &str, path: &str) -> Result<GregorianDate> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
    {
        return Err(invalid(path, value, "expected exact YYYY-MM-DD syntax"));
    }
    let year = parse_digits(&bytes[0..4]);
    let month = parse_digits(&bytes[5..7]);
    let day = parse_digits(&bytes[8..10]);
    if year == 0 || !(1..=12).contains(&month) {
        return Err(invalid(
            path,
            value,
            "year must be 0001..9999 and month 01..12",
        ));
    }
    let maximum = days_in_month(year, month);
    if day == 0 || day > maximum {
        return Err(invalid(
            path,
            value,
            &format!("day must be 01..={maximum:02} for that month"),
        ));
    }
    Ok(GregorianDate { year, month, day })
}

impl GregorianDate {
    pub(super) fn day_of_year(self) -> u32 {
        (1..self.month)
            .map(|month| days_in_month(self.year, month))
            .sum::<u32>()
            + self.day
    }

    pub(super) fn days_in_year(self) -> u32 {
        if is_leap_year(self.year) { 366 } else { 365 }
    }

    pub(super) fn weekday_monday_zero(self) -> u32 {
        const OFFSETS: [u32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let mut year = self.year;
        if self.month < 3 {
            year -= 1;
        }
        let sunday_zero = (year + year / 4 - year / 100
            + year / 400
            + OFFSETS[(self.month - 1) as usize]
            + self.day)
            % 7;
        (sunday_zero + 6) % 7
    }
}

fn parse_digits(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0, |value, byte| value * 10 + u32::from(byte - b'0'))
}

const fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

const fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn invalid(path: &str, value: &str, detail: &str) -> calyx_core::CalyxError {
    structured_runtime_error(
        CALYX_STRUCTURED_DATE_INVALID,
        format!("structured date {path:?} value {value:?} is invalid: {detail}"),
    )
}
