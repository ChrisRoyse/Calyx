pub const ENGINE: &str = "turso";
// The public `turso` 0.7.0 crate resolves its engine packages at 0.7.1.
// The SQL handshake changes from the package version to the file-header
// compatibility version after initialization, so the persisted contract uses
// this Cargo.lock-pinned engine version explicitly.
pub const ENGINE_VERSION: &str = "0.7.1";
pub const SCHEMA_VERSION: i64 = 2;

pub const CREATE_SCHEMA: &str = r#"
CREATE TABLE calyx_schema (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL,
    engine TEXT NOT NULL,
    engine_version TEXT NOT NULL,
    journal_mode TEXT NOT NULL,
    initialized_at_ms INTEGER NOT NULL
);
CREATE TABLE calyx_datasets (
    dataset_id TEXT PRIMARY KEY CHECK (length(dataset_id) BETWEEN 1 AND 255),
    vault_id TEXT NOT NULL UNIQUE,
    vault_salt BLOB NOT NULL CHECK (length(vault_salt) = 32),
    panel_version INTEGER NOT NULL CHECK (panel_version > 0),
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE calyx_records (
    dataset_id TEXT NOT NULL,
    record_id TEXT NOT NULL CHECK (length(record_id) BETWEEN 1 AND 255),
    source_revision INTEGER NOT NULL CHECK (source_revision > 0),
    modality_json TEXT NOT NULL,
    content BLOB NOT NULL CHECK (length(content) > 0),
    content_hash BLOB NOT NULL CHECK (length(content_hash) = 32),
    source_hash BLOB NOT NULL CHECK (length(source_hash) = 32),
    slot_set_hash BLOB NOT NULL CHECK (length(slot_set_hash) = 32),
    created_at_secs INTEGER NOT NULL CHECK (created_at_secs >= 0),
    metadata_json BLOB NOT NULL,
    scalars_json BLOB NOT NULL,
    anchors_json BLOB NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (dataset_id, record_id),
    UNIQUE (dataset_id, content_hash),
    FOREIGN KEY (dataset_id) REFERENCES calyx_datasets(dataset_id) ON DELETE RESTRICT
);
CREATE TABLE calyx_record_slots (
    dataset_id TEXT NOT NULL,
    record_id TEXT NOT NULL,
    slot_id INTEGER NOT NULL CHECK (slot_id BETWEEN 0 AND 65535),
    lens_id TEXT NOT NULL CHECK (length(lens_id) = 32),
    vector_json BLOB NOT NULL,
    vector_hash BLOB NOT NULL CHECK (length(vector_hash) = 32),
    PRIMARY KEY (dataset_id, record_id, slot_id),
    FOREIGN KEY (dataset_id, record_id)
        REFERENCES calyx_records(dataset_id, record_id) ON DELETE CASCADE
);
CREATE TABLE calyx_projections (
    dataset_id TEXT NOT NULL,
    record_id TEXT NOT NULL,
    source_revision INTEGER NOT NULL CHECK (source_revision > 0),
    source_hash BLOB NOT NULL CHECK (length(source_hash) = 32),
    slot_set_hash BLOB NOT NULL CHECK (length(slot_set_hash) = 32),
    cx_id TEXT NOT NULL CHECK (length(cx_id) = 32),
    aster_snapshot INTEGER NOT NULL CHECK (aster_snapshot >= 0),
    projected_at_ms INTEGER NOT NULL,
    PRIMARY KEY (dataset_id, record_id),
    UNIQUE (dataset_id, cx_id),
    FOREIGN KEY (dataset_id, record_id)
        REFERENCES calyx_records(dataset_id, record_id) ON DELETE RESTRICT
);
CREATE TABLE calyx_projection_failures (
    id INTEGER PRIMARY KEY,
    dataset_id TEXT NOT NULL,
    record_id TEXT NOT NULL,
    source_revision INTEGER NOT NULL,
    error_code TEXT NOT NULL,
    message TEXT NOT NULL,
    remediation TEXT NOT NULL,
    failed_at_ms INTEGER NOT NULL,
    resolved_at_ms INTEGER,
    resolved_by_snapshot INTEGER,
    CHECK (
        (resolved_at_ms IS NULL AND resolved_by_snapshot IS NULL) OR
        (resolved_at_ms IS NOT NULL AND resolved_by_snapshot IS NOT NULL)
    )
);
CREATE INDEX calyx_slots_lens_idx
    ON calyx_record_slots(dataset_id, lens_id, record_id);
CREATE INDEX calyx_projection_revision_idx
    ON calyx_projections(dataset_id, source_revision, record_id);
CREATE INDEX calyx_projection_failures_dataset_idx
    ON calyx_projection_failures(dataset_id, resolved_at_ms, id);
"#;
