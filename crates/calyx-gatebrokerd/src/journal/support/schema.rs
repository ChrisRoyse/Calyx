use crate::journal::db::{JournalDb, QueryExecutor, db_params};

use super::super::{JOURNAL_APPLICATION_ID, JOURNAL_SCHEMA_VERSION, JournalError};
use super::util::sql;

pub(in crate::journal) fn preflight_schema(connection: &JournalDb) -> Result<(), JournalError> {
    let version_row = connection
        .query_one("PRAGMA user_version", db_params!()?)
        .map_err(|source| sql("preflight schema version", source))?;
    let version: i64 = version_row
        .get(0)
        .map_err(|source| sql("decode preflight schema version", source))?;
    let application_row = connection
        .query_one("PRAGMA application_id", db_params!()?)
        .map_err(|source| sql("preflight application id", source))?;
    let application_id: i64 = application_row
        .get(0)
        .map_err(|source| sql("decode preflight application id", source))?;
    match version {
        0 => {
            let objects = unversioned_schema_objects(connection)?;
            if !objects.is_empty() || application_id != 0 {
                return Err(JournalError::Corrupt(format!(
                    "unversioned journal is not empty: schema_objects={objects:?}, application_id={application_id}"
                )));
            }
            Ok(())
        }
        JOURNAL_SCHEMA_VERSION if application_id == JOURNAL_APPLICATION_ID => {
            validate_schema(connection)
        }
        JOURNAL_SCHEMA_VERSION => Err(JournalError::Corrupt(format!(
            "journal application id {application_id} does not match {JOURNAL_APPLICATION_ID}"
        ))),
        _ => Err(JournalError::Corrupt(format!(
            "schema version {version} is unsupported"
        ))),
    }
}

pub(in crate::journal) fn configure(connection: &JournalDb) -> Result<(), JournalError> {
    let mode_row = connection
        .query_one("PRAGMA journal_mode='mvcc'", db_params!()?)
        .map_err(|source| sql("enable MVCC", source))?;
    let mode: String = mode_row
        .get(0)
        .map_err(|source| sql("decode journal mode", source))?;
    if !mode.eq_ignore_ascii_case("mvcc") {
        return Err(JournalError::Durability(format!(
            "Turso refused MVCC mode and returned {mode:?}"
        )));
    }
    connection
        .execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")
        .map_err(|source| sql("set durability pragmas", source))
}

pub(in crate::journal) fn initialize_schema(connection: &JournalDb) -> Result<(), JournalError> {
    let version_row = connection
        .query_one("PRAGMA user_version", db_params!()?)
        .map_err(|source| sql("read schema version", source))?;
    let version: i64 = version_row
        .get(0)
        .map_err(|source| sql("decode schema version", source))?;
    if version != 0 && version != JOURNAL_SCHEMA_VERSION {
        return Err(JournalError::Corrupt(format!(
            "schema version {version} is unsupported"
        )));
    }
    let application_row = connection
        .query_one("PRAGMA application_id", db_params!()?)
        .map_err(|source| sql("read application id before schema initialization", source))?;
    let application_id: i64 = application_row
        .get(0)
        .map_err(|source| sql("decode application id", source))?;
    if version == 0 {
        let objects = unversioned_schema_objects(connection)?;
        if !objects.is_empty() || application_id != 0 {
            return Err(JournalError::Corrupt(format!(
                "unversioned journal is not empty: schema_objects={objects:?}, application_id={application_id}"
            )));
        }
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE runs(
               run_id TEXT PRIMARY KEY CHECK(length(run_id)=32 AND run_id NOT GLOB '*[^0-9a-f]*'),
               request_id TEXT NOT NULL UNIQUE CHECK(length(request_id)=36),
               run_token TEXT NOT NULL CHECK(length(run_token)=64 AND run_token NOT GLOB '*[^0-9a-f]*'),
               profile TEXT NOT NULL CHECK(length(profile) BETWEEN 1 AND 64),
               owner_uid INTEGER NOT NULL CHECK(owner_uid BETWEEN 0 AND 4294967295),
               owner_pid INTEGER NOT NULL CHECK(owner_pid BETWEEN 1 AND 2147483647),
               owner_starttime TEXT NOT NULL CHECK(length(owner_starttime) BETWEEN 1 AND 20 AND owner_starttime NOT GLOB '*[^0-9]*'),
               state TEXT NOT NULL CHECK(state IN ('active','succeeded','failed','aborted')),
               detail TEXT CHECK(detail IS NULL OR length(detail) BETWEEN 1 AND 2048),
               created_ms INTEGER NOT NULL CHECK(created_ms >= 0),
               updated_ms INTEGER NOT NULL CHECK(updated_ms >= created_ms)
             ) STRICT;
             CREATE TABLE operations(
               request_id TEXT PRIMARY KEY CHECK(length(request_id)=36),
               request_hash BLOB NOT NULL CHECK(length(request_hash)=32),
               request_json BLOB NOT NULL CHECK(length(request_json) BETWEEN 1 AND 65536),
               verb TEXT NOT NULL CHECK(length(verb) BETWEEN 1 AND 64),
               run_id TEXT REFERENCES runs(run_id),
               state TEXT NOT NULL CHECK(state IN ('pending','succeeded','failed')),
               response_json BLOB,
               error_code TEXT CHECK(error_code IS NULL OR length(error_code) BETWEEN 1 AND 96),
               created_ms INTEGER NOT NULL CHECK(created_ms >= 0),
               updated_ms INTEGER NOT NULL CHECK(updated_ms >= created_ms),
               CHECK(
                 (state='pending' AND response_json IS NULL AND error_code IS NULL) OR
                 (state='succeeded' AND response_json IS NOT NULL AND length(response_json) BETWEEN 1 AND 65536 AND error_code IS NULL) OR
                 (state='failed' AND response_json IS NOT NULL AND length(response_json) BETWEEN 1 AND 65536 AND error_code IS NOT NULL)
               )
             ) STRICT;
             CREATE TABLE object_transactions(
               object_id TEXT PRIMARY KEY CHECK(length(object_id)=32 AND object_id NOT GLOB '*[^0-9a-f]*'),
               request_id TEXT NOT NULL UNIQUE CHECK(length(request_id)=36),
               run_id TEXT NOT NULL REFERENCES runs(run_id),
               role TEXT NOT NULL CHECK(length(role) BETWEEN 1 AND 64),
               root_alias TEXT NOT NULL CHECK(length(root_alias) BETWEEN 1 AND 64),
               leaf TEXT NOT NULL CHECK(length(leaf) BETWEEN 1 AND 240),
               state TEXT NOT NULL CHECK(state IN ('intent','prepared','published','committed','delete_intent','quarantined','deleted','mismatch_preserved','failed')),
               device TEXT,
               inode TEXT,
               owner_uid INTEGER CHECK(owner_uid BETWEEN 0 AND 4294967295),
               owner_gid INTEGER CHECK(owner_gid BETWEEN 0 AND 4294967295),
               mode INTEGER CHECK(mode BETWEEN 0 AND 4095),
               mount_id INTEGER,
               handle_type INTEGER,
               handle BLOB CHECK(handle IS NULL OR length(handle) BETWEEN 1 AND 128),
               quarantine_name TEXT CHECK(quarantine_name IS NULL OR length(quarantine_name) BETWEEN 1 AND 128),
               error_code TEXT CHECK(error_code IS NULL OR length(error_code) BETWEEN 1 AND 96),
               detail TEXT CHECK(detail IS NULL OR length(detail) BETWEEN 1 AND 2048),
               created_ms INTEGER NOT NULL CHECK(created_ms >= 0),
               updated_ms INTEGER NOT NULL CHECK(updated_ms >= created_ms),
               CHECK(
                 (device IS NULL AND inode IS NULL AND owner_uid IS NULL AND owner_gid IS NULL AND mode IS NULL AND mount_id IS NULL AND handle_type IS NULL AND handle IS NULL) OR
                 (device IS NOT NULL AND length(device) BETWEEN 1 AND 20 AND device NOT GLOB '*[^0-9]*' AND inode IS NOT NULL AND length(inode) BETWEEN 1 AND 20 AND inode NOT GLOB '*[^0-9]*' AND owner_uid IS NOT NULL AND owner_gid IS NOT NULL AND mode IS NOT NULL AND mount_id IS NOT NULL AND handle_type IS NOT NULL AND handle IS NOT NULL)
               ),
               CHECK(state IN ('intent','failed') OR device IS NOT NULL),
               CHECK(state!='intent' OR (device IS NULL AND quarantine_name IS NULL AND error_code IS NULL)),
               CHECK(state!='failed' OR (device IS NULL AND quarantine_name IS NULL)),
               CHECK(state NOT IN ('quarantined','deleted') OR quarantine_name IS NOT NULL),
               CHECK(state NOT IN ('mismatch_preserved','failed') OR error_code IS NOT NULL)
             ) STRICT;
             CREATE TABLE journal_events(
               event_id INTEGER PRIMARY KEY AUTOINCREMENT,
               object_id TEXT NOT NULL REFERENCES object_transactions(object_id),
               from_state TEXT CHECK(from_state IS NULL OR from_state IN ('intent','prepared','published','committed','delete_intent','quarantined')),
               to_state TEXT NOT NULL CHECK(to_state IN ('intent','prepared','published','committed','delete_intent','quarantined','deleted','mismatch_preserved','failed')),
               error_code TEXT CHECK(error_code IS NULL OR length(error_code) BETWEEN 1 AND 96),
               detail TEXT CHECK(detail IS NULL OR length(detail) BETWEEN 1 AND 2048),
               at_ms INTEGER NOT NULL CHECK(at_ms >= 0),
               CHECK((from_state IS NULL AND to_state='intent') OR from_state IS NOT NULL)
             ) STRICT;
             CREATE TABLE stages(
               stage_id TEXT PRIMARY KEY CHECK(length(stage_id)=32 AND stage_id NOT GLOB '*[^0-9a-f]*'),
               request_id TEXT NOT NULL UNIQUE CHECK(length(request_id)=36),
               run_id TEXT NOT NULL REFERENCES runs(run_id),
               label TEXT NOT NULL CHECK(length(label) BETWEEN 1 AND 96),
               state TEXT NOT NULL CHECK(state IN ('intent','running','succeeded','failed')),
               unit TEXT NOT NULL CHECK(length(unit) BETWEEN 1 AND 255),
               slice_unit TEXT NOT NULL CHECK(length(slice_unit) BETWEEN 1 AND 255),
               worker_user TEXT NOT NULL CHECK(length(worker_user) BETWEEN 1 AND 64 AND worker_user NOT GLOB '*[^0-9A-Za-z_-]*'),
               worker_uid INTEGER NOT NULL CHECK(worker_uid BETWEEN 1 AND 4294967295),
               invocation_id TEXT CHECK(invocation_id IS NULL OR (length(invocation_id)=32 AND invocation_id NOT GLOB '*[^0-9a-f]*')),
               control_group TEXT CHECK(control_group IS NULL OR (length(control_group) BETWEEN 1 AND 4096 AND substr(control_group,1,1)='/')),
               slice_control_group TEXT CHECK(slice_control_group IS NULL OR (length(slice_control_group) BETWEEN 1 AND 4096 AND substr(slice_control_group,1,1)='/')),
               control_group_device TEXT CHECK(control_group_device IS NULL OR (length(control_group_device) BETWEEN 1 AND 20 AND control_group_device NOT GLOB '*[^0-9]*')),
               control_group_inode TEXT CHECK(control_group_inode IS NULL OR (length(control_group_inode) BETWEEN 1 AND 20 AND control_group_inode NOT GLOB '*[^0-9]*')),
               slice_control_group_device TEXT CHECK(slice_control_group_device IS NULL OR (length(slice_control_group_device) BETWEEN 1 AND 20 AND slice_control_group_device NOT GLOB '*[^0-9]*')),
               slice_control_group_inode TEXT CHECK(slice_control_group_inode IS NULL OR (length(slice_control_group_inode) BETWEEN 1 AND 20 AND slice_control_group_inode NOT GLOB '*[^0-9]*')),
               main_pid INTEGER CHECK(main_pid IS NULL OR main_pid BETWEEN 1 AND 2147483647),
               exit_status INTEGER CHECK(exit_status IS NULL OR exit_status BETWEEN -2147483648 AND 2147483647),
               created_ms INTEGER NOT NULL CHECK(created_ms >= 0),
               updated_ms INTEGER NOT NULL CHECK(updated_ms >= created_ms),
               CHECK(
                 (state='intent' AND invocation_id IS NULL AND control_group IS NULL AND slice_control_group IS NULL AND control_group_device IS NULL AND control_group_inode IS NULL AND slice_control_group_device IS NULL AND slice_control_group_inode IS NULL AND main_pid IS NULL AND exit_status IS NULL) OR
                 (state='running' AND invocation_id IS NOT NULL AND control_group IS NOT NULL AND slice_control_group IS NOT NULL AND control_group_device IS NOT NULL AND control_group_inode IS NOT NULL AND slice_control_group_device IS NOT NULL AND slice_control_group_inode IS NOT NULL AND main_pid IS NOT NULL AND exit_status IS NULL) OR
                 (state='succeeded' AND invocation_id IS NOT NULL AND control_group IS NOT NULL AND slice_control_group IS NOT NULL AND control_group_device IS NOT NULL AND control_group_inode IS NOT NULL AND slice_control_group_device IS NOT NULL AND slice_control_group_inode IS NOT NULL AND main_pid IS NOT NULL AND exit_status=0) OR
                 (state='failed' AND exit_status IS NOT NULL AND exit_status!=0 AND ((invocation_id IS NULL AND control_group IS NULL AND slice_control_group IS NULL AND control_group_device IS NULL AND control_group_inode IS NULL AND slice_control_group_device IS NULL AND slice_control_group_inode IS NULL AND main_pid IS NULL) OR (invocation_id IS NOT NULL AND control_group IS NOT NULL AND slice_control_group IS NOT NULL AND control_group_device IS NOT NULL AND control_group_inode IS NOT NULL AND slice_control_group_device IS NOT NULL AND slice_control_group_inode IS NOT NULL AND main_pid IS NOT NULL)))
               )
             ) STRICT;
             CREATE TABLE stage_events(
               event_id INTEGER PRIMARY KEY AUTOINCREMENT,
               stage_id TEXT NOT NULL REFERENCES stages(stage_id),
               from_state TEXT CHECK(from_state IS NULL OR from_state IN ('intent','running')),
               to_state TEXT NOT NULL CHECK(to_state IN ('intent','running','succeeded','failed')),
               detail TEXT CHECK(detail IS NULL OR length(detail) BETWEEN 1 AND 2048),
               at_ms INTEGER NOT NULL CHECK(at_ms >= 0),
               CHECK((from_state IS NULL AND to_state='intent') OR from_state IS NOT NULL)
             ) STRICT;
             CREATE UNIQUE INDEX runs_one_active ON runs((1)) WHERE state='active';
             CREATE INDEX object_transactions_recovery ON object_transactions(state,created_ms);
             CREATE INDEX stages_recovery ON stages(state,created_ms);
             CREATE INDEX operations_recovery ON operations(state,created_ms);
             PRAGMA application_id=1129924418;
             PRAGMA user_version=4;
             COMMIT;"
        ).map_err(|source| sql("initialize schema", source))?;
    } else if application_id != JOURNAL_APPLICATION_ID {
        return Err(JournalError::Corrupt(format!(
            "journal application id {application_id} does not match {JOURNAL_APPLICATION_ID}"
        )));
    }
    validate_schema(connection)
}

fn unversioned_schema_objects(connection: &JournalDb) -> Result<Vec<String>, JournalError> {
    let rows = connection
        .query(
            "SELECT name FROM pragma_table_list WHERE schema='main' \
             AND type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'turso_%' \
             AND name <> '__turso_internal_mvcc_meta' \
             ORDER BY name",
            db_params!()?,
        )
        .map_err(|source| sql("inspect unversioned database", source))?;
    rows.into_iter()
        .map(|row| {
            row.get::<String>(0)
                .map_err(|source| sql("decode unversioned schema object", source))
        })
        .collect()
}

pub(in crate::journal) fn validate_schema(connection: &JournalDb) -> Result<(), JournalError> {
    const TABLES: &[(&str, &[&str])] = &[
        (
            "journal_events",
            &[
                "event_id",
                "object_id",
                "from_state",
                "to_state",
                "error_code",
                "detail",
                "at_ms",
            ],
        ),
        (
            "object_transactions",
            &[
                "object_id",
                "request_id",
                "run_id",
                "role",
                "root_alias",
                "leaf",
                "state",
                "device",
                "inode",
                "owner_uid",
                "owner_gid",
                "mode",
                "mount_id",
                "handle_type",
                "handle",
                "quarantine_name",
                "error_code",
                "detail",
                "created_ms",
                "updated_ms",
            ],
        ),
        (
            "operations",
            &[
                "request_id",
                "request_hash",
                "request_json",
                "verb",
                "run_id",
                "state",
                "response_json",
                "error_code",
                "created_ms",
                "updated_ms",
            ],
        ),
        (
            "runs",
            &[
                "run_id",
                "request_id",
                "run_token",
                "profile",
                "owner_uid",
                "owner_pid",
                "owner_starttime",
                "state",
                "detail",
                "created_ms",
                "updated_ms",
            ],
        ),
        (
            "stage_events",
            &[
                "event_id",
                "stage_id",
                "from_state",
                "to_state",
                "detail",
                "at_ms",
            ],
        ),
        (
            "stages",
            &[
                "stage_id",
                "request_id",
                "run_id",
                "label",
                "state",
                "unit",
                "slice_unit",
                "worker_user",
                "worker_uid",
                "invocation_id",
                "control_group",
                "slice_control_group",
                "control_group_device",
                "control_group_inode",
                "slice_control_group_device",
                "slice_control_group_inode",
                "main_pid",
                "exit_status",
                "created_ms",
                "updated_ms",
            ],
        ),
    ];
    let actual_tables = connection
        .query(
            "SELECT name FROM pragma_table_list WHERE schema='main' AND type='table' \
             AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'turso_%' \
             AND name NOT IN (\
                '__turso_internal_mvcc_meta', \
                '__turso_internal_seq___turso_internal_autoincrement_journal_events', \
                '__turso_internal_seq___turso_internal_autoincrement_stage_events'\
             ) ORDER BY name",
            db_params!()?,
        )
        .map_err(|source| sql("query schema tables", source))?
        .into_iter()
        .map(|row| {
            row.get::<String>(0)
                .map_err(|source| sql("read schema table", source))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let expected_tables = TABLES
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect::<Vec<_>>();
    if actual_tables != expected_tables {
        return Err(JournalError::Corrupt(format!(
            "journal tables differ from schema v{JOURNAL_SCHEMA_VERSION}: expected {expected_tables:?}, actual {actual_tables:?}"
        )));
    }
    for (table, expected_columns) in TABLES {
        let pragma = format!("PRAGMA table_info('{table}')");
        let actual_columns = connection
            .query(&pragma, db_params!()?)
            .map_err(|source| sql("query schema columns", source))?
            .into_iter()
            .map(|row| {
                row.get::<String>(1)
                    .map_err(|source| sql("read schema column", source))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if actual_columns != *expected_columns {
            return Err(JournalError::Corrupt(format!(
                "journal table {table} columns differ from schema v{JOURNAL_SCHEMA_VERSION}: expected {expected_columns:?}, actual {actual_columns:?}"
            )));
        }
        let strict_row = connection
            .query_one(
                "SELECT strict FROM pragma_table_list WHERE schema='main' AND name=?1",
                db_params![*table]?,
            )
            .map_err(|source| sql("verify strict journal table", source))?;
        let strict: i64 = strict_row
            .get(0)
            .map_err(|source| sql("decode strict journal table", source))?;
        if strict != 1 {
            return Err(JournalError::Corrupt(format!(
                "journal table {table} is not STRICT"
            )));
        }
    }
    let actual_indexes = connection
        .query(
            "SELECT name FROM pragma_index_list('runs') WHERE origin='c' \
             UNION SELECT name FROM pragma_index_list('operations') WHERE origin='c' \
             UNION SELECT name FROM pragma_index_list('object_transactions') WHERE origin='c' \
             UNION SELECT name FROM pragma_index_list('stages') WHERE origin='c' \
             ORDER BY name",
            db_params!()?,
        )
        .map_err(|source| sql("query schema indexes", source))?
        .into_iter()
        .map(|row| {
            row.get::<String>(0)
                .map_err(|source| sql("read schema index", source))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let expected_indexes = vec![
        "object_transactions_recovery".to_owned(),
        "operations_recovery".to_owned(),
        "runs_one_active".to_owned(),
        "stages_recovery".to_owned(),
    ];
    if actual_indexes != expected_indexes {
        return Err(JournalError::Corrupt(format!(
            "journal indexes differ from schema v{JOURNAL_SCHEMA_VERSION}: expected {expected_indexes:?}, actual {actual_indexes:?}"
        )));
    }
    let foreign_key_violations = connection
        .query("PRAGMA foreign_key_check", db_params!()?)
        .map_err(|source| sql("check journal foreign keys", source))?;
    if let Some(first) = foreign_key_violations.first() {
        let table = first
            .get::<String>(0)
            .map_err(|source| sql("decode foreign key violation table", source))?;
        let rowid = first
            .get::<i64>(1)
            .map_err(|source| sql("decode foreign key violation rowid", source))?;
        let parent = first
            .get::<String>(2)
            .map_err(|source| sql("decode foreign key violation parent", source))?;
        let foreign_key = first
            .get::<i64>(3)
            .map_err(|source| sql("decode foreign key violation id", source))?;
        return Err(JournalError::Corrupt(format!(
            "journal contains {} foreign-key violations; first is table={table:?}, rowid={rowid}, parent={parent:?}, foreign_key={foreign_key}",
            foreign_key_violations.len()
        )));
    }
    Ok(())
}
