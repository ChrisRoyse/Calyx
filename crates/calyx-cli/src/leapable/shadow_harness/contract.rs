use std::collections::BTreeMap;

use calyx_core::CalyxError;
use rusqlite::Connection;
use serde::Serialize;

use super::{CALYX_PG_CONTRACT_VIOLATION, error};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct PgContractReport {
    pub tables: BTreeMap<String, Vec<ColumnSpec>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ColumnSpec {
    pub name: String,
    pub declared_type: String,
}

pub(crate) fn verify_pg_contract_for(conn: &Connection) -> Result<PgContractReport, CalyxError> {
    let mut tables = BTreeMap::new();
    tables.insert(
        "creator_databases".to_string(),
        require_columns(
            conn,
            "creator_databases",
            &[
                ("id", "INTEGER"),
                ("database_name", "TEXT"),
                ("created_at", "TEXT"),
            ],
        )?,
    );
    tables.insert(
        "queries".to_string(),
        require_columns(
            conn,
            "queries",
            &[
                ("id", "INTEGER"),
                ("database_name", "TEXT"),
                ("query_text", "TEXT"),
            ],
        )?,
    );
    Ok(PgContractReport { tables })
}

fn require_columns(
    conn: &Connection,
    table: &str,
    expected: &[(&str, &str)],
) -> Result<Vec<ColumnSpec>, CalyxError> {
    let actual = table_columns(conn, table)?;
    for (name, declared_type) in expected {
        let Some(found) = actual.iter().find(|column| column.name == *name) else {
            return Err(contract_violation(format!("{table} missing column {name}")));
        };
        if !found.declared_type.eq_ignore_ascii_case(declared_type) {
            return Err(contract_violation(format!(
                "{table}.{name} type {} != {declared_type}",
                found.declared_type
            )));
        }
    }
    Ok(actual)
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<ColumnSpec>, CalyxError> {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|error| contract_violation(format!("inspect {table}: {error}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ColumnSpec {
                name: row.get::<_, String>(1)?,
                declared_type: row.get::<_, String>(2)?,
            })
        })
        .map_err(|error| contract_violation(format!("read {table} columns: {error}")))?;
    let columns = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| contract_violation(format!("collect {table} columns: {error}")))?;
    if columns.is_empty() {
        return Err(contract_violation(format!("{table} table is absent")));
    }
    Ok(columns)
}

fn contract_violation(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_PG_CONTRACT_VIOLATION,
        message,
        "keep Leapable control-plane contract table names, columns, and declared types unchanged",
    )
}
