use std::fs;
use std::path::Path;

use crate::error::CliResult;
use crate::output::print_hex_dump;
use crate::{budget_readback, tripwire_readback};

pub(crate) fn readback_hex(path: &Path) -> CliResult {
    let bytes = fs::read(path)?;
    print_hex_dump(0, &bytes);
    Ok(())
}

pub(crate) fn parse_i64(value: &str) -> Result<i64, String> {
    value
        .parse::<i64>()
        .map_err(|error| format!("invalid i64 value {value}: {error}"))
}

pub(crate) fn parse_i32(value: &str) -> Result<i32, String> {
    value
        .parse::<i32>()
        .map_err(|error| format!("invalid i32 value {value}: {error}"))
}

pub(crate) fn readback_config(name: &str, vault: &Path) -> CliResult {
    match name {
        "tripwire" => tripwire_readback::readback_tripwire_config(vault),
        "budget" => budget_readback::readback_budget_config(vault),
        _ => Err(format!("unknown config readback: {name}").into()),
    }
}

#[cfg(test)]
pub(crate) fn hex_lines(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(32)
        .map(|chunk| {
            let mut line = String::with_capacity(chunk.len() * 2);
            for byte in chunk {
                line.push(hex_digit(byte >> 4));
                line.push(hex_digit(byte & 0x0f));
            }
            line
        })
        .collect()
}

#[cfg(test)]
fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
