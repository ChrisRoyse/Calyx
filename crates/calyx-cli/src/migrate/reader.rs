use std::path::Path;

use calyx_core::Result;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, Row};

use super::errors;

#[derive(Clone, Debug, PartialEq)]
pub struct ChunkRow {
    pub rowid: i64,
    pub chunk_id: String,
    pub database_name: String,
    pub content: Vec<u8>,
    pub embedding: Vec<f32>,
}

impl ChunkRow {
    pub fn identity_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_len_prefixed(&mut out, self.database_name.as_bytes());
        put_len_prefixed(&mut out, self.chunk_id.as_bytes());
        put_len_prefixed(&mut out, &self.content);
        out
    }

    pub fn content_hash(&self) -> [u8; 32] {
        *blake3::hash(&self.content).as_bytes()
    }

    pub fn pointer(&self) -> String {
        format!("sqlite://chunks/{}/{}", self.database_name, self.chunk_id)
    }
}

pub fn open_sqlite(path: &Path) -> Result<Connection> {
    Connection::open(path).map_err(|err| errors::sqlite("open sqlite", err))
}

pub fn validate_schema(conn: &Connection) -> Result<()> {
    let columns = table_columns(conn)?;
    for required in ["chunk_id", "database_name", "content", "embedding"] {
        if !columns.iter().any(|column| column == required) {
            return Err(errors::schema(format!(
                "chunks table missing required column {required}"
            )));
        }
    }
    Ok(())
}

pub fn stream_rows(conn: &Connection) -> Result<Vec<ChunkRow>> {
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

pub fn read_chunk(conn: &Connection, chunk_id: &str) -> Result<ChunkRow> {
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
        return Err(errors::schema(format!("chunk_id {chunk_id} not found")));
    };
    row_from_sqlite(row)
}

fn table_columns(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(chunks)")
        .map_err(|err| errors::sqlite("inspect chunks schema", err))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| errors::sqlite("read chunks schema", err))?;
    rows.map(|row| row.map_err(|err| errors::sqlite("decode schema row", err)))
        .collect()
}

fn row_from_sqlite(row: &Row<'_>) -> Result<ChunkRow> {
    let chunk = ChunkRow {
        rowid: row
            .get(0)
            .map_err(|err| errors::sqlite("read rowid", err))?,
        chunk_id: row
            .get(1)
            .map_err(|err| errors::sqlite("read chunk_id", err))?,
        database_name: row
            .get(2)
            .map_err(|err| errors::sqlite("read database_name", err))?,
        content: value_bytes(
            row.get_ref(3)
                .map_err(|err| errors::sqlite("read content", err))?,
        )?,
        embedding: decode_embedding(value_bytes(
            row.get_ref(4)
                .map_err(|err| errors::sqlite("read embedding", err))?,
        )?)?,
    };
    if chunk.chunk_id.is_empty() || chunk.database_name.is_empty() {
        return Err(errors::schema(
            "chunk_id and database_name must be non-empty",
        ));
    }
    Ok(chunk)
}

fn value_bytes(value: ValueRef<'_>) -> Result<Vec<u8>> {
    match value {
        ValueRef::Blob(bytes) | ValueRef::Text(bytes) => Ok(bytes.to_vec()),
        _ => Err(errors::schema("content and embedding must be TEXT or BLOB")),
    }
}

fn decode_embedding(bytes: Vec<u8>) -> Result<Vec<f32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return Err(errors::embedding(format!(
            "embedding byte length {} is not positive f32 LE data",
            bytes.len()
        )));
    }
    let values = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect::<Vec<_>>();
    if values.iter().any(|value| !value.is_finite()) {
        return Err(errors::embedding("embedding contains NaN or Inf"));
    }
    Ok(values)
}

fn put_len_prefixed(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_and_raw_f32_embedding() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks VALUES('c1','db','alpha',?1)",
            [embedding_blob(&[1.0, 2.0])],
        )
        .unwrap();

        let rows = stream_rows(&conn).unwrap();

        assert_eq!(rows[0].content, b"alpha");
        assert_eq!(rows[0].embedding, vec![1.0, 2.0]);
    }

    fn embedding_blob(values: &[f32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }
}
