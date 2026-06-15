use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::CalyxError;
use serde::{Deserialize, Serialize};

use super::error;

#[cfg(test)]
pub(crate) const CALYX_PG_WRITE_ATTEMPTED: &str = "CALYX_PG_WRITE_ATTEMPTED";
pub(crate) const CALYX_VAULT_NOT_IN_PG: &str = "CALYX_VAULT_NOT_IN_PG";
const CALYX_PG_SNAPSHOT_INCOMPLETE: &str = "CALYX_PG_SNAPSHOT_INCOMPLETE";
pub(crate) const REQUIRED_TABLES: &[&str] = &[
    "creator_databases",
    "queries",
    "billing",
    "marketplace",
    "outbox",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PgConn {
    ReadOnlyPsql {
        conninfo: String,
    },
    DumpDir {
        root: PathBuf,
    },
    #[cfg(test)]
    WriteCapableForTest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PgSnapshot {
    pub(crate) vault_name: String,
    pub(crate) taken_at_ms: u64,
    pub(crate) tables: Vec<TableHash>,
    pub(crate) snapshot_blake3: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TableHash {
    pub(crate) table: String,
    pub(crate) dump_path: PathBuf,
    pub(crate) bytes_len: usize,
    pub(crate) row_count: usize,
    pub(crate) blake3: String,
    pub(crate) contains_vault_name: bool,
}

pub(crate) fn snapshot_pg_state(
    pg_conn: &PgConn,
    vault_name: &str,
    out_dir: &Path,
) -> Result<PgSnapshot, CalyxError> {
    if vault_name.is_empty() {
        return Err(snapshot_error("vault_name must not be empty"));
    }
    match pg_conn {
        #[cfg(test)]
        PgConn::WriteCapableForTest => Err(error(
            CALYX_PG_WRITE_ATTEMPTED,
            "write-capable PostgreSQL connection rejected",
            "open the control-plane connection through PgConn::ReadOnlyPsql",
        )),
        PgConn::DumpDir { root } => snapshot_from_dump_dir(root, vault_name, out_dir),
        PgConn::ReadOnlyPsql { conninfo } => snapshot_from_psql(conninfo, vault_name, out_dir),
    }
}

fn snapshot_from_dump_dir(
    root: &Path,
    vault_name: &str,
    out_dir: &Path,
) -> Result<PgSnapshot, CalyxError> {
    fs::create_dir_all(out_dir)
        .map_err(|err| snapshot_error(format!("create {}: {err}", out_dir.display())))?;
    let mut tables = Vec::with_capacity(REQUIRED_TABLES.len());
    for table in REQUIRED_TABLES {
        let source = root.join(format!("{table}.dump"));
        if !source.is_file() {
            return Err(snapshot_error(format!("missing {}", source.display())));
        }
        let bytes = fs::read(&source)
            .map_err(|err| snapshot_error(format!("read {}: {err}", source.display())))?;
        let target = out_dir.join(format!("{table}.dump"));
        fs::write(&target, &bytes)
            .map_err(|err| snapshot_error(format!("write {}: {err}", target.display())))?;
        tables.push(table_hash(table, &target, &bytes, vault_name));
    }
    finish_snapshot(vault_name, tables)
}

fn snapshot_from_psql(
    conninfo: &str,
    vault_name: &str,
    out_dir: &Path,
) -> Result<PgSnapshot, CalyxError> {
    fs::create_dir_all(out_dir)
        .map_err(|err| snapshot_error(format!("create {}: {err}", out_dir.display())))?;
    let mut tables = Vec::with_capacity(REQUIRED_TABLES.len());
    for table in REQUIRED_TABLES {
        let bytes = psql_table_dump(conninfo, table, vault_name)?;
        let target = out_dir.join(format!("{table}.dump"));
        fs::write(&target, &bytes)
            .map_err(|err| snapshot_error(format!("write {}: {err}", target.display())))?;
        tables.push(table_hash(table, &target, &bytes, vault_name));
    }
    finish_snapshot(vault_name, tables)
}

fn finish_snapshot(vault_name: &str, tables: Vec<TableHash>) -> Result<PgSnapshot, CalyxError> {
    let creator = tables
        .iter()
        .find(|table| table.table == "creator_databases")
        .ok_or_else(|| snapshot_error("creator_databases snapshot missing"))?;
    if creator.row_count == 0 || !creator.contains_vault_name {
        return Err(error(
            CALYX_VAULT_NOT_IN_PG,
            format!("vault_name {vault_name} absent from creator_databases"),
            "pass the verbatim Leapable database_name present in PostgreSQL",
        ));
    }
    let mut hasher = blake3::Hasher::new();
    for table in &tables {
        hasher.update(table.table.as_bytes());
        hasher.update(table.blake3.as_bytes());
        hasher.update(&(table.bytes_len as u64).to_be_bytes());
        hasher.update(&(table.row_count as u64).to_be_bytes());
    }
    Ok(PgSnapshot {
        vault_name: vault_name.to_string(),
        taken_at_ms: now_ms(),
        tables,
        snapshot_blake3: hasher.finalize().to_string(),
    })
}

fn table_hash(table: &str, path: &Path, bytes: &[u8], vault_name: &str) -> TableHash {
    TableHash {
        table: table.to_string(),
        dump_path: path.to_path_buf(),
        bytes_len: bytes.len(),
        row_count: bytes.lines().count(),
        blake3: blake3::hash(bytes).to_string(),
        contains_vault_name: contains_bytes(bytes, vault_name.as_bytes()),
    }
}

fn psql_table_dump(conninfo: &str, table: &str, vault_name: &str) -> Result<Vec<u8>, CalyxError> {
    let where_clause = match table {
        "creator_databases" | "queries" => {
            format!("WHERE database_name = '{}'", sql_literal(vault_name))
        }
        _ => String::new(),
    };
    let sql = format!(
        "BEGIN READ ONLY; \
         SET LOCAL default_transaction_read_only = on; \
         COPY (SELECT row_to_json(t)::text FROM (SELECT * FROM {table} {where_clause}) t ORDER BY 1) TO STDOUT; \
         COMMIT;"
    );
    let output = Command::new("psql")
        .arg("-X")
        .arg("-q")
        .arg("-A")
        .arg("-v")
        .arg("ON_ERROR_STOP=1")
        .arg(conninfo)
        .arg("-c")
        .arg(sql)
        .output()
        .map_err(|err| snapshot_error(format!("spawn psql: {err}")))?;
    if !output.status.success() {
        return Err(snapshot_error(format!(
            "psql table={table} exited {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output.stdout)
}

fn sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

fn snapshot_error(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_PG_SNAPSHOT_INCOMPLETE,
        message,
        "read the control-plane table dumps and retry with a complete read-only snapshot",
    )
}

trait ByteLines {
    fn lines(&self) -> std::str::Lines<'_>;
}

impl ByteLines for [u8] {
    fn lines(&self) -> std::str::Lines<'_> {
        std::str::from_utf8(self).unwrap_or("").lines()
    }
}
