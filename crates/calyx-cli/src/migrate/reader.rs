use std::path::Path;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, Row};

use super::errors;
use crate::error::{CliError, CliResult};

const GTE_EMBEDDING_DIM: usize = 768;
const GTE_EMBEDDING_BYTES: usize = GTE_EMBEDDING_DIM * std::mem::size_of::<f32>();

#[derive(Clone, Debug, PartialEq)]
pub struct ChunkRow {
    pub row_num: u64,
    pub chunk_id: String,
    pub database_name: String,
    pub content: Vec<u8>,
    pub embedding: Vec<f32>,
}

impl ChunkRow {
    pub fn content_hash(&self) -> [u8; 32] {
        *blake3::hash(&self.content).as_bytes()
    }

    pub fn pointer(&self) -> String {
        format!("sqlite://chunks/{}/{}", self.database_name, self.chunk_id)
    }
}

pub fn open_sqlite(path: &Path) -> CliResult<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Connection::open_with_flags(path, flags)
        .map_err(|err| CliError::io(format!("open sqlite {}: {err}", path.display())))
}

pub fn validate_schema(conn: &Connection) -> CliResult {
    let columns = table_columns(conn)?;
    for required in ["chunk_id", "database_name", "content", "embedding"] {
        if !columns.iter().any(|column| column == required) {
            return Err(
                errors::schema(format!("chunks table missing required column {required}")).into(),
            );
        }
    }
    Ok(())
}

pub fn row_count(conn: &Connection) -> CliResult<u64> {
    validate_schema(conn)?;
    let count = conn
        .query_row("SELECT COUNT(*) FROM chunks", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|err| errors::sqlite("count chunks", err))?;
    u64::try_from(count)
        .map_err(|_| errors::schema(format!("SQLite row count {count} is negative")).into())
}

pub fn stream_rows(conn: &Connection) -> CliResult<Vec<ChunkRow>> {
    validate_schema(conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT rowid, chunk_id, database_name, content, embedding \
             FROM chunks ORDER BY rowid",
        )
        .map_err(|err| errors::sqlite("prepare chunk scan", err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| errors::sqlite("query chunks", err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| errors::sqlite("read chunk row", err))?
    {
        out.push(row_from_sqlite(row)?);
    }
    Ok(out)
}

pub fn read_chunk(conn: &Connection, chunk_id: &str) -> CliResult<ChunkRow> {
    validate_schema(conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT rowid, chunk_id, database_name, content, embedding \
             FROM chunks WHERE chunk_id = ?1 ORDER BY rowid LIMIT 1",
        )
        .map_err(|err| errors::sqlite("prepare chunk read", err))?;
    let mut rows = stmt
        .query([chunk_id])
        .map_err(|err| errors::sqlite("query chunk", err))?;
    let Some(row) = rows
        .next()
        .map_err(|err| errors::sqlite("read chunk", err))?
    else {
        return Err(errors::schema(format!("chunk_id {chunk_id} not found")).into());
    };
    row_from_sqlite(row)
}

fn table_columns(conn: &Connection) -> CliResult<Vec<String>> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(chunks)")
        .map_err(|err| errors::sqlite("inspect chunks schema", err))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| errors::sqlite("read chunks schema", err))?;
    rows.map(|row| row.map_err(|err| errors::sqlite("decode schema row", err)))
        .collect::<calyx_core::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn row_from_sqlite(row: &Row<'_>) -> CliResult<ChunkRow> {
    let rowid: i64 = row
        .get(0)
        .map_err(|err| errors::sqlite("read rowid", err))?;
    let row_num = u64::try_from(rowid)
        .map_err(|_| errors::schema(format!("chunks rowid {rowid} is negative")))?;
    Ok(ChunkRow {
        row_num,
        chunk_id: text_field(
            row.get_ref(1)
                .map_err(|err| errors::sqlite(&format!("read chunk_id at row {row_num}"), err))?,
            "chunk_id",
            row_num,
        )?,
        database_name: text_field(
            row.get_ref(2).map_err(|err| {
                errors::sqlite(&format!("read database_name at row {row_num}"), err)
            })?,
            "database_name",
            row_num,
        )?,
        content: value_bytes(
            row.get_ref(3)
                .map_err(|err| errors::sqlite(&format!("read content at row {row_num}"), err))?,
            "content",
            row_num,
        )?,
        embedding: decode_embedding(
            value_bytes(
                row.get_ref(4).map_err(|err| {
                    errors::sqlite(&format!("read embedding at row {row_num}"), err)
                })?,
                "embedding",
                row_num,
            )?,
            row_num,
        )?,
    })
}

fn text_field(value: ValueRef<'_>, field: &str, row_num: u64) -> CliResult<String> {
    let bytes = value_bytes(value, field, row_num)?;
    std::str::from_utf8(&bytes)
        .map(str::to_string)
        .map_err(|err| {
            errors::schema(format!(
                "row {row_num} {field} is not valid UTF-8: {err}; raw_hex={}",
                super::manifest::hex_encode(&bytes)
            ))
            .into()
        })
}

fn value_bytes(value: ValueRef<'_>, field: &str, row_num: u64) -> CliResult<Vec<u8>> {
    match value {
        ValueRef::Blob(bytes) | ValueRef::Text(bytes) => Ok(bytes.to_vec()),
        _ => Err(errors::schema(format!("row {row_num} {field} must be TEXT or BLOB")).into()),
    }
}

fn decode_embedding(bytes: Vec<u8>, row_num: u64) -> CliResult<Vec<f32>> {
    if bytes.len() != GTE_EMBEDDING_BYTES {
        return Err(errors::embedding(format!(
            "row {row_num} embedding byte length {} expected {GTE_EMBEDDING_BYTES}",
            bytes.len(),
        ))
        .into());
    }
    let values = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect::<Vec<_>>();
    if values.iter().any(|value| !value.is_finite()) {
        return Err(
            errors::embedding(format!("row {row_num} embedding contains NaN or Inf")).into(),
        );
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_rows_in_rowid_order_and_preserves_empty_identity_fields() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
            [],
        )
        .unwrap();
        for (chunk_id, database_name, content, first) in [
            ("c3", "db", "gamma", 3.0),
            ("", "db", "empty chunk", 2.0),
            ("c1", "", "empty db", 1.0),
        ] {
            conn.execute(
                "INSERT INTO chunks VALUES(?1,?2,?3,?4)",
                (chunk_id, database_name, content, embedding_blob(first)),
            )
            .unwrap();
        }

        let rows = stream_rows(&conn).unwrap();

        assert_eq!(row_count(&conn).unwrap(), 3);
        assert_eq!(
            rows.iter().map(|row| row.row_num).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(rows[0].content, b"gamma");
        assert_eq!(rows[0].embedding.len(), GTE_EMBEDDING_DIM);
        assert_eq!(rows[0].embedding[0], 3.0);
        assert_eq!(rows[1].chunk_id, "");
        assert_eq!(rows[2].database_name, "");
    }

    #[test]
    fn missing_required_schema_column_fails_closed() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT)",
            [],
        )
        .unwrap();

        let error = validate_schema(&conn).unwrap_err();

        assert_eq!(error.code(), errors::CALYX_MIGRATE_SQLITE_SCHEMA);
        assert!(error.message().contains("embedding"));
        assert!(error.remediation().contains("Leapable Vault SQLite DB"));
    }

    #[test]
    fn exact_gte_embedding_blob_decodes_first_little_endian_float() {
        let conn = one_row_db("c1", "db", "alpha", embedding_blob(1.0));
        let rows = stream_rows(&conn).unwrap();

        assert_eq!(rows[0].embedding.len(), GTE_EMBEDDING_DIM);
        assert_eq!(rows[0].embedding[0], 1.0);
    }

    #[test]
    fn wrong_embedding_size_reports_row_number() {
        let conn = one_row_db("c1", "db", "alpha", vec![0_u8; GTE_EMBEDDING_BYTES - 4]);

        let error = stream_rows(&conn).unwrap_err();

        assert_eq!(error.code(), errors::CALYX_MIGRATE_EMBEDDING_FORMAT);
        assert!(error.message().contains("row 1"));
        assert!(error.message().contains("3068"));
    }

    #[test]
    fn empty_chunks_table_streams_zero_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
            [],
        )
        .unwrap();

        assert_eq!(row_count(&conn).unwrap(), 0);
        assert_eq!(stream_rows(&conn).unwrap(), Vec::new());
    }

    #[test]
    fn non_utf8_chunk_id_fails_with_row_number() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE chunks(chunk_id BLOB,database_name TEXT,content TEXT,embedding BLOB)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks VALUES(?1,'db','alpha',?2)",
            (vec![0xff, 0xfe], embedding_blob(1.0)),
        )
        .unwrap();

        let error = stream_rows(&conn).unwrap_err();

        assert_eq!(error.code(), errors::CALYX_MIGRATE_SQLITE_SCHEMA);
        assert!(error.message().contains("row 1"));
        assert!(error.message().contains("raw_hex=fffe"));
    }

    #[test]
    fn null_embedding_fails_with_row_number() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO chunks VALUES('c1','db','alpha',NULL)", [])
            .unwrap();

        let error = stream_rows(&conn).unwrap_err();

        assert_eq!(error.code(), errors::CALYX_MIGRATE_SQLITE_SCHEMA);
        assert!(error.message().contains("row 1"));
        assert!(error.message().contains("embedding"));
    }

    #[test]
    fn non_utf8_database_name_reports_raw_bytes() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE chunks(chunk_id TEXT,database_name BLOB,content TEXT,embedding BLOB)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks VALUES('c1',?1,'alpha',?2)",
            (vec![0xff, 0x00], embedding_blob(1.0)),
        )
        .unwrap();

        let error = stream_rows(&conn).unwrap_err();

        assert_eq!(error.code(), errors::CALYX_MIGRATE_SQLITE_SCHEMA);
        assert!(error.message().contains("row 1"));
        assert!(error.message().contains("database_name"));
        assert!(error.message().contains("raw_hex=ff00"));
    }

    fn one_row_db(
        chunk_id: &str,
        database_name: &str,
        content: &str,
        embedding: Vec<u8>,
    ) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks VALUES(?1,?2,?3,?4)",
            (chunk_id, database_name, content, embedding),
        )
        .unwrap();
        conn
    }

    fn embedding_blob(first: f32) -> Vec<u8> {
        std::iter::once(first)
            .chain((1..GTE_EMBEDDING_DIM).map(|idx| idx as f32 / 10.0))
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }
}
