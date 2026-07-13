use super::*;

pub(super) fn preflight_schema(connection: &Connection) -> Result<(), JournalError> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|source| sql("preflight schema version", source))?;
    let application_id: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(|source| sql("preflight application id", source))?;
    match version {
        0 => {
            let objects: i64 = connection
                .query_row(
                    "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|source| sql("preflight unversioned database", source))?;
            if objects != 0 || application_id != 0 {
                return Err(JournalError::Corrupt(format!(
                    "unversioned journal is not empty: schema_objects={objects}, application_id={application_id}"
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

pub(super) fn configure(connection: &Connection) -> Result<(), JournalError> {
    let mode: String = connection
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .map_err(|source| sql("enable WAL", source))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(JournalError::Durability(format!(
            "SQLite refused WAL mode and returned {mode:?}"
        )));
    }
    connection.execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON;")
        .map_err(|source| sql("set durability pragmas", source))
}

pub(super) fn initialize_schema(connection: &Connection) -> Result<(), JournalError> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|source| sql("read schema version", source))?;
    if version != 0 && version != JOURNAL_SCHEMA_VERSION {
        return Err(JournalError::Corrupt(format!(
            "schema version {version} is unsupported"
        )));
    }
    let application_id: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(|source| sql("read application id before schema initialization", source))?;
    if version == 0 {
        let objects: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )
            .map_err(|source| sql("inspect unversioned database", source))?;
        if objects != 0 || application_id != 0 {
            return Err(JournalError::Corrupt(format!(
                "unversioned journal is not empty: schema_objects={objects}, application_id={application_id}"
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
             PRAGMA user_version=3;
             COMMIT;"
        ).map_err(|source| sql("initialize schema", source))?;
    } else if application_id != JOURNAL_APPLICATION_ID {
        return Err(JournalError::Corrupt(format!(
            "journal application id {application_id} does not match {JOURNAL_APPLICATION_ID}"
        )));
    }
    validate_schema(connection)
}

pub(super) fn validate_schema(connection: &Connection) -> Result<(), JournalError> {
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
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(|source| sql("prepare schema table inspection", source))?;
    let actual_tables = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|source| sql("query schema tables", source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| sql("read schema tables", source))?;
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
        let mut statement = connection
            .prepare(&pragma)
            .map_err(|source| sql("prepare schema column inspection", source))?;
        let actual_columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|source| sql("query schema columns", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| sql("read schema columns", source))?;
        if actual_columns != *expected_columns {
            return Err(JournalError::Corrupt(format!(
                "journal table {table} columns differ from schema v{JOURNAL_SCHEMA_VERSION}: expected {expected_columns:?}, actual {actual_columns:?}"
            )));
        }
        let strict: i64 = connection
            .query_row(
                "SELECT strict FROM pragma_table_list WHERE schema='main' AND name=?1",
                [*table],
                |row| row.get(0),
            )
            .map_err(|source| sql("verify strict journal table", source))?;
        if strict != 1 {
            return Err(JournalError::Corrupt(format!(
                "journal table {table} is not STRICT"
            )));
        }
    }
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type='index' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(|source| sql("prepare schema index inspection", source))?;
    let actual_indexes = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|source| sql("query schema indexes", source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| sql("read schema indexes", source))?;
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
    let foreign_key_violations: i64 = connection
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .map_err(|source| sql("check journal foreign keys", source))?;
    if foreign_key_violations != 0 {
        return Err(JournalError::Corrupt(format!(
            "journal contains {foreign_key_violations} foreign-key violations"
        )));
    }
    Ok(())
}

pub(super) fn require_active_run(
    transaction: &rusqlite::Transaction<'_>,
    run_id: &RunId,
    operation: &str,
) -> Result<(), JournalError> {
    let state = transaction
        .query_row(
            "SELECT state FROM runs WHERE run_id=?1",
            [run_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|source| sql("read run authority", source))?;
    match state.as_deref() {
        Some("active") => Ok(()),
        Some(state) => Err(JournalError::InvalidMetadata(format!(
            "{operation} requires active run {run_id}, found {state}"
        ))),
        None => Err(JournalError::InvalidMetadata(format!(
            "{operation} references absent run {run_id}"
        ))),
    }
}

pub(super) fn query_text_column(
    connection: &Connection,
    query: &'static str,
    operation: &'static str,
) -> Result<Vec<String>, JournalError> {
    let mut statement = connection
        .prepare(query)
        .map_err(|source| sql(operation, source))?;
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|source| sql(operation, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| sql(operation, source))
}

pub(super) fn validate_stage_event_chain(
    connection: &Connection,
    record: &StageRecord,
) -> Result<(), JournalError> {
    let mut statement = connection
        .prepare(
            "SELECT event_id,from_state,to_state FROM stage_events WHERE stage_id=?1 ORDER BY event_id",
        )
        .map_err(|source| sql("prepare stage event integrity query", source))?;
    let events = statement
        .query_map([record.intent.stage_id.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|source| sql("query stage event integrity", source))?;
    let mut previous = None;
    for event in events {
        let (event_id, from, to) =
            event.map_err(|source| sql("read stage event integrity", source))?;
        let from = from.map(|value| StageState::parse(&value)).transpose()?;
        let to = StageState::parse(&to)?;
        if from != previous {
            return Err(JournalError::Corrupt(format!(
                "stage {} event {event_id} starts at {from:?}, expected {previous:?}",
                record.intent.stage_id
            )));
        }
        if let Some(from) = from
            && !stage_transition_allowed(from, to)
        {
            return Err(JournalError::Corrupt(format!(
                "stage {} event {event_id} records forbidden transition {from:?} -> {to:?}",
                record.intent.stage_id
            )));
        }
        previous = Some(to);
    }
    if previous != Some(record.state) {
        return Err(JournalError::Corrupt(format!(
            "stage {} event chain ends at {previous:?}, row is {:?}",
            record.intent.stage_id, record.state
        )));
    }
    Ok(())
}

pub(super) fn read_transaction(
    connection: &Connection,
    object_id: &ObjectId,
) -> Result<Option<TransactionRecord>, JournalError> {
    connection.query_row(
        "SELECT request_id,run_id,role,root_alias,leaf,state,device,inode,owner_uid,owner_gid,mode,mount_id,handle_type,handle,quarantine_name,error_code,detail,created_ms,updated_ms FROM object_transactions WHERE object_id=?1",
        [object_id.as_str()], |row| transaction_from_row(object_id.clone(), row),
    ).optional().map_err(|source| sql("read transaction", source))?.transpose()
}

pub(super) fn transaction_from_row(
    object_id: ObjectId,
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<TransactionRecord, JournalError>> {
    let request_id = RequestId::new(row.get::<_, String>(0)?);
    let run_id = RunId::new(row.get::<_, String>(1)?);
    let role = RoleName::new(row.get::<_, String>(2)?);
    let root_alias = RootAlias::new(row.get::<_, String>(3)?);
    let leaf = LeafName::new(row.get::<_, String>(4)?);
    let state = TransactionState::parse(&row.get::<_, String>(5)?);
    let device: Option<String> = row.get(6)?;
    let identity = if let Some(device) = device {
        let inode: String = row.get(7)?;
        let opaque = OpaqueHandle::new(row.get(11)?, row.get(12)?, row.get(13)?)
            .map_err(|error| JournalError::Corrupt(error.to_string()));
        Some(opaque.and_then(|opaque| {
            Ok(ObjectIdentity {
                device: parse_u64(&device, "device")?,
                inode: parse_u64(&inode, "inode")?,
                owner_uid: row
                    .get(8)
                    .map_err(|e| JournalError::Corrupt(e.to_string()))?,
                owner_gid: row
                    .get(9)
                    .map_err(|e| JournalError::Corrupt(e.to_string()))?,
                mode: row
                    .get(10)
                    .map_err(|e| JournalError::Corrupt(e.to_string()))?,
                opaque,
            })
        }))
    } else {
        None
    };
    Ok((|| {
        let record = TransactionRecord {
            intent: IntentRecord {
                object_id,
                request_id: request_id.map_err(corrupt_protocol)?,
                run_id: run_id.map_err(corrupt_protocol)?,
                role: role.map_err(corrupt_protocol)?,
                root_alias: root_alias.map_err(corrupt_protocol)?,
                leaf: leaf.map_err(corrupt_protocol)?,
            },
            state: state?,
            identity: identity.transpose()?,
            quarantine_name: row.get(14).map_err(corrupt_sql)?,
            error_code: row.get(15).map_err(corrupt_sql)?,
            detail: row.get(16).map_err(corrupt_sql)?,
            created_ms: row.get(17).map_err(corrupt_sql)?,
            updated_ms: row.get(18).map_err(corrupt_sql)?,
        };
        validate_transaction_record(&record)?;
        Ok(record)
    })())
}

fn validate_transaction_record(record: &TransactionRecord) -> Result<(), JournalError> {
    let has_identity = record.identity.is_some();
    let has_quarantine = record.quarantine_name.is_some();
    let has_error = record.error_code.is_some();
    let valid = match record.state {
        TransactionState::Intent => !has_identity && !has_quarantine && !has_error,
        TransactionState::Prepared
        | TransactionState::Published
        | TransactionState::Committed
        | TransactionState::DeleteIntent => has_identity && !has_error,
        TransactionState::Quarantined | TransactionState::Deleted => {
            has_identity && has_quarantine && !has_error
        }
        TransactionState::MismatchPreserved => has_identity && has_error,
        TransactionState::Failed => !has_identity && !has_quarantine && has_error,
    };
    if !valid {
        return Err(JournalError::Corrupt(format!(
            "object {} state {:?} has inconsistent identity, quarantine, or error metadata",
            record.intent.object_id, record.state
        )));
    }
    if let Some(name) = record.quarantine_name.as_deref()
        && name != format!("q-{}", record.intent.object_id)
    {
        return Err(JournalError::Corrupt(format!(
            "object {} has unexpected quarantine name {name:?}",
            record.intent.object_id
        )));
    }
    Ok(())
}

pub(super) fn run_from_row(
    run_id: RunId,
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<RunRecord, JournalError>> {
    let request = RequestId::new(row.get::<_, String>(0)?);
    let token = RunToken::new(row.get::<_, String>(1)?);
    let profile = ProfileName::new(row.get::<_, String>(2)?);
    let starttime: String = row.get(5)?;
    let state: String = row.get(6)?;
    Ok((|| {
        Ok(RunRecord {
            intent: RunIntent {
                run_id,
                request_id: request.map_err(corrupt_protocol)?,
                run_token: token.map_err(corrupt_protocol)?,
                profile: profile.map_err(corrupt_protocol)?,
                owner_uid: row.get(3).map_err(corrupt_sql)?,
                owner_pid: row.get(4).map_err(corrupt_sql)?,
                owner_starttime: parse_u64(&starttime, "owner_starttime")?,
            },
            state: RunState::parse(&state)?,
            created_ms: row.get(7).map_err(corrupt_sql)?,
            updated_ms: row.get(8).map_err(corrupt_sql)?,
            detail: row.get(9).map_err(corrupt_sql)?,
        })
    })())
}

pub(super) fn validate_transition(
    from: TransactionState,
    to: TransactionState,
    update: &TransitionUpdate,
) -> Result<(), JournalError> {
    if !transaction_transition_allowed(from, to) {
        return Err(JournalError::InvalidMetadata(format!(
            "transition {from:?} -> {to:?} is forbidden"
        )));
    }
    if to == TransactionState::Prepared && update.identity.is_none() {
        return Err(JournalError::InvalidMetadata(
            "Prepared requires an object identity".into(),
        ));
    }
    if to == TransactionState::Quarantined
        && update.quarantine_name.as_deref().is_none_or(str::is_empty)
    {
        return Err(JournalError::InvalidMetadata(
            "Quarantined requires quarantine_name".into(),
        ));
    }
    if update.identity.is_some()
        && !matches!(
            to,
            TransactionState::Prepared | TransactionState::Quarantined
        )
    {
        return Err(JournalError::InvalidMetadata(
            "identity metadata is only valid for Prepared or Quarantined".into(),
        ));
    }
    if update.quarantine_name.is_some()
        && !matches!(
            to,
            TransactionState::Quarantined | TransactionState::MismatchPreserved
        )
    {
        return Err(JournalError::InvalidMetadata(
            "quarantine_name is only valid for Quarantined or MismatchPreserved".into(),
        ));
    }
    if matches!(
        to,
        TransactionState::MismatchPreserved | TransactionState::Failed
    ) && update.error_code.as_deref().is_none_or(str::is_empty)
    {
        return Err(JournalError::InvalidMetadata(
            "failure state requires error_code".into(),
        ));
    }
    if !matches!(
        to,
        TransactionState::MismatchPreserved | TransactionState::Failed
    ) && update.error_code.is_some()
    {
        return Err(JournalError::InvalidMetadata(
            "error_code is only valid for a failure state".into(),
        ));
    }
    validate_optional_text("quarantine_name", update.quarantine_name.as_deref(), 128)?;
    validate_optional_text("error_code", update.error_code.as_deref(), 96)?;
    validate_optional_text("detail", update.detail.as_deref(), 2_048)
}

pub(super) fn transaction_transition_allowed(from: TransactionState, to: TransactionState) -> bool {
    matches!(
        (from, to),
        (
            TransactionState::Intent,
            TransactionState::Prepared | TransactionState::Failed
        ) | (
            TransactionState::Prepared,
            TransactionState::Published | TransactionState::Quarantined
        ) | (
            TransactionState::Published,
            TransactionState::Committed
                | TransactionState::DeleteIntent
                | TransactionState::Quarantined
                | TransactionState::MismatchPreserved
        ) | (TransactionState::Committed, TransactionState::DeleteIntent)
            | (
                TransactionState::DeleteIntent,
                TransactionState::Quarantined | TransactionState::MismatchPreserved
            )
            | (
                TransactionState::Quarantined,
                TransactionState::Deleted | TransactionState::MismatchPreserved
            )
    )
}

pub(super) fn identity_fields(
    identity: &ObjectIdentity,
) -> (String, String, u32, u32, u32, i32, i32, Vec<u8>) {
    (
        identity.device.to_string(),
        identity.inode.to_string(),
        identity.owner_uid,
        identity.owner_gid,
        identity.mode,
        identity.opaque.mount_id(),
        identity.opaque.handle_type(),
        identity.opaque.bytes().to_vec(),
    )
}

pub(super) fn validate_journal_path(path: &Path) -> Result<(), JournalError> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(JournalError::InvalidPath(format!(
            "{} must be an absolute file path",
            path.display()
        )));
    }
    let parent = path
        .parent()
        .ok_or_else(|| JournalError::InvalidPath("journal has no parent".into()))?;
    validate_journal_parent(parent)
}

pub(super) fn prepare_journal_file(path: &Path) -> Result<bool, JournalError> {
    let result = prepare_journal_file_platform(path);
    match result {
        Ok(file) => {
            file.sync_all().map_err(|source| JournalError::FileSync {
                path: path.into(),
                source,
            })?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            validate_journal_file(path)?;
            Ok(false)
        }
        Err(source) => Err(JournalError::FileSync {
            path: path.into(),
            source,
        }),
    }
}

#[cfg(unix)]
pub(super) fn prepare_journal_file_platform(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
pub(super) fn prepare_journal_file_platform(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(unix)]
pub(super) fn validate_journal_parent(path: &Path) -> Result<(), JournalError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = path
        .symlink_metadata()
        .map_err(|source| JournalError::FileSync {
            path: path.into(),
            source,
        })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
    {
        return Err(JournalError::InvalidPath(format!(
            "parent {} must be a real directory owned by broker uid with no group/other write bits",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn validate_journal_parent(path: &Path) -> Result<(), JournalError> {
    let metadata = path
        .symlink_metadata()
        .map_err(|source| JournalError::FileSync {
            path: path.into(),
            source,
        })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(JournalError::InvalidPath(format!(
            "parent {} is not a real directory",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn validate_journal_file(path: &Path) -> Result<(), JournalError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = path
        .symlink_metadata()
        .map_err(|source| JournalError::FileSync {
            path: path.into(),
            source,
        })?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(JournalError::InvalidPath(format!(
            "{} must be a regular, non-hardlinked, broker-owned 0600 file",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn validate_journal_file(path: &Path) -> Result<(), JournalError> {
    let metadata = path
        .symlink_metadata()
        .map_err(|source| JournalError::FileSync {
            path: path.into(),
            source,
        })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(JournalError::InvalidPath(format!(
            "{} is not a regular journal file",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn sync_parent(path: &Path) -> Result<(), JournalError> {
    let parent = path.parent().expect("validated parent");
    std::fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|source| JournalError::FileSync {
            path: parent.into(),
            source,
        })
}

#[cfg(not(unix))]
pub(super) fn sync_parent(_path: &Path) -> Result<(), JournalError> {
    Ok(())
}

#[cfg(unix)]
pub(super) fn sync_file(path: &Path) -> Result<(), JournalError> {
    std::fs::OpenOptions::new()
        .read(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|source| JournalError::FileSync {
            path: path.into(),
            source,
        })
}

#[cfg(not(unix))]
pub(super) fn sync_file(_path: &Path) -> Result<(), JournalError> {
    // SQLite's FULL synchronous checkpoint issues FlushFileBuffers on Windows.
    Ok(())
}

pub(super) fn now_ms() -> Result<i64, JournalError> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| JournalError::Durability(format!("system clock precedes epoch: {error}")))?
        .as_millis();
    i64::try_from(value)
        .map_err(|_| JournalError::Durability("timestamp exceeds SQLite range".into()))
}

pub(super) fn parse_u64(value: &str, field: &str) -> Result<u64, JournalError> {
    value
        .parse()
        .map_err(|error| JournalError::Corrupt(format!("invalid {field} {value:?}: {error}")))
}

pub(super) fn validate_optional_text(
    field: &str,
    value: Option<&str>,
    maximum: usize,
) -> Result<(), JournalError> {
    if value.is_some_and(|value| {
        value.is_empty() || value.len() > maximum || value.contains(['\0', '\r', '\n'])
    }) {
        return Err(JournalError::InvalidMetadata(format!(
            "{field} is empty, oversized, or contains control bytes"
        )));
    }
    Ok(())
}

pub(super) fn sql(operation: &'static str, source: rusqlite::Error) -> JournalError {
    JournalError::Sql { operation, source }
}
pub(super) fn corrupt_protocol(error: crate::protocol::ProtocolError) -> JournalError {
    JournalError::Corrupt(error.to_string())
}
pub(super) fn corrupt_sql(error: rusqlite::Error) -> JournalError {
    JournalError::Corrupt(error.to_string())
}
