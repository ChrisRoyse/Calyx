use std::collections::BTreeMap;

use rusqlite::Connection;
use rusqlite::types::ValueRef;

use super::{CALYX_ROUND_TRIP_MISMATCH, SourceRow, cli_to_calyx, error};
use crate::error::CliError;
use crate::migrate::manifest::hex_decode;
use crate::migrate::reader::stream_rows;

const METADATA_TEXT_HASH: &str = "text_hash";

pub(super) fn source_rows(conn: &Connection) -> Result<Vec<SourceRow>, calyx_core::CalyxError> {
    let rows = stream_rows(conn).map_err(cli_to_calyx)?;
    let hashes = text_hashes_by_rowid(conn)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let text_hash = hashes
                .get(&row.row_num)
                .copied()
                .unwrap_or_else(|| row.content_hash());
            SourceRow { row, text_hash }
        })
        .collect())
}

fn text_hashes_by_rowid(
    conn: &Connection,
) -> Result<BTreeMap<u64, [u8; 32]>, calyx_core::CalyxError> {
    if !chunks_has_text_hash(conn)? {
        return Ok(BTreeMap::new());
    }
    let mut stmt = conn
        .prepare("SELECT rowid, text_hash FROM chunks ORDER BY rowid")
        .map_err(|err| cli_to_calyx(CliError::io(format!("prepare text_hash scan: {err}"))))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| cli_to_calyx(CliError::io(format!("query text_hash scan: {err}"))))?;
    let mut out = BTreeMap::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| cli_to_calyx(CliError::io(format!("read text_hash row: {err}"))))?
    {
        let rowid: i64 = row
            .get(0)
            .map_err(|err| cli_to_calyx(CliError::io(format!("decode text_hash rowid: {err}"))))?;
        let rowid = u64::try_from(rowid).map_err(|_| {
            error(
                CALYX_ROUND_TRIP_MISMATCH,
                "negative sqlite rowid in text_hash scan",
                "repair the source SQLite rowid table before verifying round-trip",
            )
        })?;
        let hash = decode_hash_value(
            row.get_ref(1)
                .map_err(|err| cli_to_calyx(CliError::io(format!("read text_hash: {err}"))))?,
            rowid,
        )?;
        out.insert(rowid, hash);
    }
    Ok(out)
}

fn chunks_has_text_hash(conn: &Connection) -> Result<bool, calyx_core::CalyxError> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(chunks)")
        .map_err(|err| cli_to_calyx(CliError::io(format!("inspect chunks schema: {err}"))))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| cli_to_calyx(CliError::io(format!("read chunks schema: {err}"))))?;
    for column in columns {
        let column =
            column.map_err(|err| cli_to_calyx(CliError::io(format!("decode column: {err}"))))?;
        if column == METADATA_TEXT_HASH {
            return Ok(true);
        }
    }
    Ok(false)
}

fn decode_hash_value(value: ValueRef<'_>, rowid: u64) -> Result<[u8; 32], calyx_core::CalyxError> {
    let bytes = match value {
        ValueRef::Blob(bytes) | ValueRef::Text(bytes) => bytes,
        _ => {
            return Err(error(
                CALYX_ROUND_TRIP_MISMATCH,
                format!("row {rowid} text_hash must be TEXT or BLOB"),
                "store text_hash as 32 raw bytes or 64 lowercase hex characters",
            ));
        }
    };
    if bytes.len() == 32 {
        return Ok(bytes.try_into().expect("32 bytes"));
    }
    let text = std::str::from_utf8(bytes).map_err(|err| {
        error(
            CALYX_ROUND_TRIP_MISMATCH,
            format!("row {rowid} text_hash is not UTF-8 hex or 32 raw bytes: {err}"),
            "store text_hash as 32 raw bytes or 64 lowercase hex characters",
        )
    })?;
    let decoded = hex_decode(text.trim()).map_err(|err| {
        error(
            CALYX_ROUND_TRIP_MISMATCH,
            format!("row {rowid} text_hash hex invalid: {}", err.message),
            "store text_hash as 32 raw bytes or 64 lowercase hex characters",
        )
    })?;
    decoded.try_into().map_err(|bytes: Vec<u8>| {
        error(
            CALYX_ROUND_TRIP_MISMATCH,
            format!(
                "row {rowid} text_hash decoded to {} bytes, expected 32",
                bytes.len()
            ),
            "store text_hash as 32 raw bytes or 64 lowercase hex characters",
        )
    })
}
