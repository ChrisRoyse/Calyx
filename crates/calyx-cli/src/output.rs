//! Canonical stdout emitters shared by every subcommand.
//!
//! Three shapes cover the CLI's output needs:
//! * [`print_json`] — machine-parseable single value for pipelines/automation.
//! * [`print_table`] — aligned human-readable columns for interactive use.
//! * [`print_hex_dump`] — byte-exact rows in `xxd -g 1` layout so an FSV reader
//!   can cross-verify the raw bytes residing in the vault against `xxd` output.
//!
//! All three write to stdout; errors are the sole concern of [`crate::error`]
//! on stderr. Keeping success output and error output on separate streams is
//! the dual-consumer contract: a pipe captures clean data on stdout while an
//! operator/agent reads the structured envelope on stderr.
//!
//! Each emitter is a thin `println!` wrapper over a pure line-builder
//! (`json_line`, `table_lines`, `hex_dump_lines`) so the exact bytes written
//! can be asserted directly in tests without capturing stdout.

use serde::Serialize;

use crate::error::{CliError, CliResult};

/// Bytes per hex-dump row (matches `xxd` default width).
const HEX_ROW: usize = 16;
/// Full-row hex-column width: 16 bytes × 2 hex chars + 15 single-space seps.
const HEX_WIDTH: usize = HEX_ROW * 2 + (HEX_ROW - 1);

/// Renders a value to its compact JSON line. Returns the serializer error
/// verbatim rather than hiding a regression behind empty output.
fn json_line<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(value)
}

/// Prints a single value as compact JSON on stdout.
pub(crate) fn print_json<T: Serialize>(value: &T) -> CliResult {
    let json = json_line(value)
        .map_err(|error| CliError::usage(format!("serialize CLI JSON output: {error}")))?;
    println!("{json}");
    Ok(())
}

/// Builds the aligned table lines (header first) for `headers`/`rows`. Column
/// widths are the max cell width per column. Ragged rows are tolerated (missing
/// cells render empty); extra cells beyond the header count are still printed.
fn table_lines(headers: &[&str], rows: &[Vec<String>]) -> Vec<String> {
    let columns = headers
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    let mut widths = vec![0usize; columns];
    for (col, header) in headers.iter().enumerate() {
        widths[col] = widths[col].max(header.len());
    }
    for row in rows {
        for (col, cell) in row.iter().enumerate() {
            widths[col] = widths[col].max(cell.len());
        }
    }

    let render = |cells: &[String]| -> String {
        (0..columns)
            .map(|col| {
                let cell = cells.get(col).map(String::as_str).unwrap_or("");
                format!("{cell:<width$}", width = widths[col])
            })
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };

    let header_cells: Vec<String> = headers.iter().map(|h| (*h).to_string()).collect();
    let mut lines = Vec::with_capacity(rows.len() + 1);
    lines.push(render(&header_cells));
    lines.extend(rows.iter().map(|row| render(row)));
    lines
}

/// Prints `rows` as a left-aligned table under `headers`.
pub(crate) fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    for line in table_lines(headers, rows) {
        println!("{line}");
    }
}

/// Builds hex-dump lines in `xxd -g 1` layout starting at `offset`:
/// `{offset:08x}  {byte byte …}  |{ascii}|`, 16 bytes per row, hex column
/// padded so the ASCII gutter aligns across partial rows. A zero-length slice
/// yields no lines. Non-printable bytes render as `.` in the ASCII gutter.
fn hex_dump_lines(offset: u64, bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(HEX_ROW)
        .enumerate()
        .map(|(row_index, chunk)| {
            let row_offset = offset + (row_index * HEX_ROW) as u64;
            let hex = chunk
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            let ascii: String = chunk
                .iter()
                .map(|&byte| {
                    if (0x20..=0x7e).contains(&byte) {
                        byte as char
                    } else {
                        '.'
                    }
                })
                .collect();
            format!("{row_offset:08x}  {hex:<HEX_WIDTH$}  |{ascii}|")
        })
        .collect()
}

/// Prints `bytes` as a hex dump (see [`hex_dump_lines`]).
pub(crate) fn print_hex_dump(offset: u64, bytes: &[u8]) {
    for line in hex_dump_lines(offset, bytes) {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_dump_first_line_matches_card_example_exactly() {
        // Synthetic known input → known line: 00 41 42 43 ⇒ ".ABC".
        let lines = hex_dump_lines(0, &[0x00u8, 0x41, 0x42, 0x43]);

        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        // offset(8) + "  " + hex padded to 47 + "  " + "|.ABC|"(6) = 65 chars.
        assert_eq!(line.len(), 8 + 2 + HEX_WIDTH + 2 + 6, "{line}");
        assert!(line.starts_with("00000000  00 41 42 43"), "{line}");
        assert!(line.ends_with("  |.ABC|"), "{line}");
    }

    #[test]
    fn hex_dump_empty_slice_yields_no_lines() {
        assert!(hex_dump_lines(0, &[]).is_empty());
    }

    #[test]
    fn hex_dump_17_bytes_spans_two_rows_with_advancing_offset() {
        let bytes: Vec<u8> = (0..17u8).collect();
        let lines = hex_dump_lines(0, &bytes);

        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("00000000  "), "{}", lines[0]);
        // Second row offset advances by exactly 16 (0x10).
        assert!(lines[1].starts_with("00000010  10"), "{}", lines[1]);
        // The lone tail byte's hex column is padded so the ASCII gutter `|`
        // opens at the same column in both rows (xxd alignment). The total
        // line lengths differ — only the hex column is padded, not the gutter.
        assert_eq!(
            lines[0].find('|'),
            lines[1].find('|'),
            "gutter must align: {lines:?}"
        );
    }

    #[test]
    fn hex_dump_all_ff_row_renders_ff_pairs_and_dot_gutter() {
        let lines = hex_dump_lines(0, &[0xffu8; 16]);

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("ff ff ff ff ff ff ff ff"), "{}", lines[0]);
        assert!(lines[0].ends_with("|................|"), "{}", lines[0]);
    }

    #[test]
    fn hex_dump_ascii_classification_boundaries_are_exact() {
        // 0x1f → '.', 0x20 (space), 0x7e (~), 0x7f (DEL) → '.', 0xff → '.'.
        let lines = hex_dump_lines(0, &[0x1f, 0x20, 0x7e, 0x7f, 0xff]);
        assert!(lines[0].ends_with("|. ~..|"), "{}", lines[0]);
    }

    #[test]
    fn hex_dump_nonzero_start_offset_is_honored() {
        let lines = hex_dump_lines(0x1234, &[0x41]);
        assert!(lines[0].starts_with("00001234  41"), "{}", lines[0]);
    }

    #[test]
    fn table_lines_align_columns_to_widest_cell() {
        let headers = ["slot", "name"];
        let rows = vec![
            vec!["0".to_string(), "text-default".to_string()],
            vec!["12".to_string(), "x".to_string()],
        ];
        let lines = table_lines(&headers, &rows);

        // Header: "slot" padded to 4, "name" padded to 12.
        assert_eq!(lines[0], "slot  name");
        // "0" padded to width 4 (widest is "slot"=4), then "text-default".
        assert_eq!(lines[1], "0     text-default");
        // "12" padded to 4, then "x" (trailing pad trimmed).
        assert_eq!(lines[2], "12    x");
    }

    #[test]
    fn table_lines_tolerate_ragged_rows() {
        let headers = ["a", "b"];
        let rows = vec![vec!["1".to_string()]]; // missing second cell
        let lines = table_lines(&headers, &rows);
        assert_eq!(lines[0], "a  b");
        assert_eq!(lines[1], "1"); // empty trailing cell trimmed
    }

    #[test]
    fn json_line_round_trips_a_known_value() {
        // Array + scalar: deterministic regardless of map key-ordering config.
        assert_eq!(json_line(&[1, 3, 7]).expect("serialize"), "[1,3,7]");
        assert_eq!(json_line(&"a\"b").expect("serialize"), r#""a\"b""#);
    }
}
