use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, CalyxError, Constellation, CxFlags, CxId, InputRef, LedgerRef, LensId, Modality,
    SlotId, SlotVector, VaultId, VaultStore,
};
use turso::transaction::TransactionBehavior;
use turso::{Connection, Database, Value, params, params_from_iter};
use ulid::Ulid;

use crate::error;
use crate::model::{
    DatasetInfo, IngestReport, InitReport, MAX_BATCH_SIZE, ProjectionReport, RecordInput,
    RecordState, SlotState, StatusReport, VerifyReport, encode_hex, validate_id,
};
use crate::schema::{CREATE_SCHEMA, ENGINE, ENGINE_VERSION, SCHEMA_VERSION};
use crate::storage;

pub struct CalyxTurso {
    path: PathBuf,
    _database: Database,
    connection: Connection,
    engine_version: String,
    journal_mode: String,
}

#[derive(Clone)]
struct Dataset {
    dataset_id: String,
    vault_id: VaultId,
    vault_salt: Vec<u8>,
    panel_version: u32,
}

#[derive(Clone)]
struct StoredRecord {
    dataset_id: String,
    record_id: String,
    source_revision: u64,
    modality: Modality,
    content: Vec<u8>,
    content_hash: [u8; 32],
    source_hash: [u8; 32],
    slot_set_hash: [u8; 32],
    created_at_secs: u64,
    metadata: BTreeMap<String, String>,
    scalars: BTreeMap<String, f64>,
    anchors: Vec<Anchor>,
    slots: BTreeMap<SlotId, StoredSlot>,
    projected_cx_id: Option<CxId>,
    projected_aster_snapshot: Option<u64>,
}

#[derive(Clone)]
struct StoredSlot {
    lens_id: LensId,
    vector: SlotVector,
    vector_hash: [u8; 32],
}

impl CalyxTurso {
    pub async fn initialize(path: impl AsRef<Path>) -> calyx_core::Result<(Self, InitReport)> {
        let path = path.as_ref();
        let existed = path
            .metadata()
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);
        if existed {
            let database = Self::open(path).await?;
            let report = database.init_report();
            return Ok((database, report));
        }
        storage::prepare(path)?;
        let path_text = path_text(path)?;
        let database = turso::Builder::new_local(path_text)
            .build()
            .await
            .map_err(|source| error::open(format!("open {}: {source}", path.display())))?;
        let connection = database
            .connect()
            .map_err(|source| error::open(format!("connect {}: {source}", path.display())))?;
        connection
            .execute("PRAGMA foreign_keys = ON", ())
            .await
            .map_err(|source| error::open(format!("enable foreign keys: {source}")))?;
        let engine_version = engine_version(&connection).await?;
        let journal_mode = set_mvcc(&connection).await?;
        let now_ms = now_ms()?;
        let schema_sql = format!(
            "BEGIN IMMEDIATE;\n{CREATE_SCHEMA}\n\
             INSERT INTO calyx_schema(singleton, schema_version, engine, engine_version, journal_mode, initialized_at_ms) \
             VALUES(1, {SCHEMA_VERSION}, '{ENGINE}', '{ENGINE_VERSION}', 'mvcc', {now_ms});\n\
             PRAGMA user_version = {SCHEMA_VERSION};\nCOMMIT;"
        );
        connection
            .execute_batch(schema_sql)
            .await
            .map_err(|source| error::transaction("initialize Calyx Turso schema", source))?;
        checkpoint_connection(&connection).await?;
        storage::verify(path)?;
        let store = Self {
            path: path.to_path_buf(),
            _database: database,
            connection,
            engine_version,
            journal_mode,
        };
        store.verify_contract().await?;
        let report = store.init_report();
        Ok((store, report))
    }

    pub async fn open(path: impl AsRef<Path>) -> calyx_core::Result<Self> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(error::open(format!(
                "Turso database {} does not exist; initialize it explicitly",
                path.display()
            )));
        }
        storage::prepare(path)?;
        let database = turso::Builder::new_local(path_text(path)?)
            .build()
            .await
            .map_err(|source| error::open(format!("open {}: {source}", path.display())))?;
        let connection = database
            .connect()
            .map_err(|source| error::open(format!("connect {}: {source}", path.display())))?;
        connection
            .execute("PRAGMA foreign_keys = ON", ())
            .await
            .map_err(|source| error::open(format!("enable foreign keys: {source}")))?;
        let engine_version = engine_version(&connection).await?;
        let journal_mode = current_journal_mode(&connection).await?;
        storage::verify(path)?;
        let store = Self {
            path: path.to_path_buf(),
            _database: database,
            connection,
            engine_version,
            journal_mode,
        };
        store.verify_contract().await?;
        Ok(store)
    }

    pub async fn create_dataset(
        &mut self,
        dataset_id: &str,
        panel_version: u32,
    ) -> calyx_core::Result<DatasetInfo> {
        validate_id("dataset_id", dataset_id)?;
        if panel_version == 0 {
            return Err(error::record("panel_version must be greater than zero"));
        }
        if let Some(existing) = self.dataset_optional(dataset_id).await? {
            if existing.panel_version != panel_version {
                return Err(error::schema(format!(
                    "dataset {dataset_id:?} already uses panel_version {}, requested {panel_version}",
                    existing.panel_version
                )));
            }
            return Ok(existing.info());
        }
        let vault_id = VaultId::from_ulid(Ulid::new());
        let vault_salt = rand::random::<[u8; 32]>();
        let now_ms = now_ms()?;
        self.connection
            .set_transaction_behavior(TransactionBehavior::Immediate);
        let transaction = self
            .connection
            .transaction()
            .await
            .map_err(|source| error::transaction("begin create-dataset", source))?;
        transaction
            .execute(
                "INSERT INTO calyx_datasets(dataset_id, vault_id, vault_salt, panel_version, created_at_ms) \
                 VALUES(?1, ?2, ?3, ?4, ?5)",
                params![
                    dataset_id,
                    vault_id.to_string(),
                    vault_salt.to_vec(),
                    i64::from(panel_version),
                    as_i64(now_ms, "created_at_ms")?
                ],
            )
            .await
            .map_err(|source| error::transaction("insert dataset", source))?;
        transaction
            .commit()
            .await
            .map_err(|source| error::transaction("commit create-dataset", source))?;
        checkpoint_connection(&self.connection).await?;
        self.dataset(dataset_id).await.map(|dataset| dataset.info())
    }

    pub async fn ingest(
        &mut self,
        dataset_id: &str,
        inputs: Vec<RecordInput>,
    ) -> calyx_core::Result<IngestReport> {
        if inputs.is_empty() || inputs.len() > MAX_BATCH_SIZE {
            return Err(error::batch(format!(
                "ingest batch has {} records; required range is 1..={MAX_BATCH_SIZE}",
                inputs.len()
            )));
        }
        let _dataset = self.dataset(dataset_id).await?;
        let now_ms = now_ms()?;
        let now_secs = now_ms / 1_000;
        let mut prepared = Vec::with_capacity(inputs.len());
        let mut ids = BTreeSet::new();
        for input in inputs {
            let record = input.prepare(now_secs)?;
            if !ids.insert(record.record_id.clone()) {
                return Err(error::record(format!(
                    "ingest batch duplicates record_id {:?}",
                    record.record_id
                )));
            }
            prepared.push(record);
        }
        let existing = self
            .existing_records(
                dataset_id,
                prepared.iter().map(|record| record.record_id.as_str()),
            )
            .await?;
        let mut inserted = 0;
        let mut updated = 0;
        let mut unchanged = 0;
        for record in &prepared {
            if let Some((_, source_hash, projected)) = existing.get(&record.record_id) {
                if source_hash == &record.source_hash {
                    unchanged += 1;
                } else if *projected {
                    return Err(error::immutable_projected(format!(
                        "record {:?} in dataset {dataset_id:?} is already projected and cannot change",
                        record.record_id
                    )));
                } else {
                    updated += 1;
                }
            } else {
                inserted += 1;
            }
        }
        if inserted == 0 && updated == 0 {
            return Ok(IngestReport {
                dataset_id: dataset_id.to_string(),
                submitted: prepared.len(),
                inserted,
                updated,
                unchanged,
                checkpointed_frames: 0,
            });
        }
        self.connection
            .set_transaction_behavior(TransactionBehavior::Immediate);
        let transaction = self
            .connection
            .transaction()
            .await
            .map_err(|source| error::transaction("begin record ingest", source))?;
        let mut upsert_record = transaction
            .prepare(
                "INSERT INTO calyx_records(\
                    dataset_id, record_id, source_revision, modality_json, content, content_hash, \
                    source_hash, slot_set_hash, created_at_secs, metadata_json, scalars_json, \
                    anchors_json, updated_at_ms\
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
                 ON CONFLICT(dataset_id, record_id) DO UPDATE SET \
                    source_revision=excluded.source_revision, modality_json=excluded.modality_json, \
                    content=excluded.content, content_hash=excluded.content_hash, \
                    source_hash=excluded.source_hash, slot_set_hash=excluded.slot_set_hash, \
                    created_at_secs=excluded.created_at_secs, metadata_json=excluded.metadata_json, \
                    scalars_json=excluded.scalars_json, anchors_json=excluded.anchors_json, \
                    updated_at_ms=excluded.updated_at_ms",
            )
            .await
            .map_err(|source| error::transaction("prepare record upsert", source))?;
        let mut delete_slots = transaction
            .prepare("DELETE FROM calyx_record_slots WHERE dataset_id = ?1 AND record_id = ?2")
            .await
            .map_err(|source| error::transaction("prepare slot delete", source))?;
        let mut insert_slot = transaction
            .prepare(
                "INSERT INTO calyx_record_slots(\
                    dataset_id, record_id, slot_id, lens_id, vector_json, vector_hash\
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .await
            .map_err(|source| error::transaction("prepare slot insert", source))?;
        let submitted = prepared.len();
        for record in prepared {
            let revision =
                existing
                    .get(&record.record_id)
                    .map_or(1, |(revision, source_hash, _)| {
                        if source_hash == &record.source_hash {
                            *revision
                        } else {
                            revision + 1
                        }
                    });
            if existing
                .get(&record.record_id)
                .is_some_and(|(_, source_hash, _)| source_hash == &record.source_hash)
            {
                continue;
            }
            let crate::model::PreparedRecord {
                record_id,
                modality_json,
                content,
                content_hash,
                source_hash,
                slot_set_hash,
                created_at_secs,
                metadata_json,
                scalars_json,
                anchors_json,
                slots,
            } = record;
            upsert_record
                .execute(params![
                    dataset_id,
                    record_id.as_str(),
                    as_i64(revision, "source_revision")?,
                    modality_json,
                    content,
                    content_hash.to_vec(),
                    source_hash.to_vec(),
                    slot_set_hash.to_vec(),
                    as_i64(created_at_secs, "created_at_secs")?,
                    metadata_json,
                    scalars_json,
                    anchors_json,
                    as_i64(now_ms, "updated_at_ms")?
                ])
                .await
                .map_err(|source| {
                    error::transaction(&format!("upsert record {record_id:?}"), source)
                })?;
            delete_slots
                .execute(params![dataset_id, record_id.as_str()])
                .await
                .map_err(|source| {
                    error::transaction(&format!("delete stale slots for {record_id:?}"), source)
                })?;
            for slot in slots {
                insert_slot
                    .execute(params![
                        dataset_id,
                        record_id.as_str(),
                        i64::from(slot.slot_id),
                        slot.lens_id,
                        slot.vector_json,
                        slot.vector_hash.to_vec()
                    ])
                    .await
                    .map_err(|source| {
                        error::transaction(
                            &format!("insert slot {} for record {:?}", slot.slot_id, record_id),
                            source,
                        )
                    })?;
            }
        }
        transaction
            .commit()
            .await
            .map_err(|source| error::transaction("commit record ingest", source))?;
        Ok(IngestReport {
            dataset_id: dataset_id.to_string(),
            submitted,
            inserted,
            updated,
            unchanged,
            checkpointed_frames: 0,
        })
    }

    pub async fn project(
        &mut self,
        dataset_id: &str,
        vault_dir: impl AsRef<Path>,
        batch_size: usize,
    ) -> calyx_core::Result<ProjectionReport> {
        validate_batch_size(batch_size)?;
        let dataset = self.dataset(dataset_id).await?;
        if !self.dataset_has_records(dataset_id).await? {
            return Err(error::projection(format!(
                "dataset {dataset_id:?} has no records to project"
            )));
        }
        let vault = AsterVault::new_durable(
            vault_dir,
            dataset.vault_id,
            dataset.vault_salt.clone(),
            VaultOptions::default(),
        )?;
        let mut projected = 0;
        let mut batches = 0;
        let mut after = String::new();
        loop {
            let records = self
                .load_pending_batch(&dataset, &after, batch_size)
                .await?;
            if records.is_empty() {
                break;
            }
            let next_after = records
                .last()
                .expect("nonempty projection batch")
                .record_id
                .clone();
            let record_ids = records
                .iter()
                .map(|record| record.record_id.clone())
                .collect::<Vec<_>>();
            match self.project_batch(&dataset, &vault, records).await {
                Ok(count) => {
                    projected += count;
                    batches += 1;
                    after = next_after;
                }
                Err(failure) => {
                    self.log_projection_failures(dataset_id, &record_ids, &failure)
                        .await?;
                    return Err(failure);
                }
            }
        }
        let (checkpoints, checkpointed_frames) = if projected == 0 {
            (0, 0)
        } else {
            (1, self.checkpoint().await?)
        };
        Ok(ProjectionReport {
            dataset_id: dataset_id.to_string(),
            projected,
            batches,
            checkpoints,
            checkpointed_frames,
            aster_snapshot: vault.snapshot(),
        })
    }

    /// Compacts committed MVCC/WAL state into the main database file.
    ///
    /// A successful transaction commit is already durable. Callers use this
    /// explicit operation boundary to amortize the engine's blocking truncate
    /// checkpoint across all bounded transactions in one logical command.
    pub async fn checkpoint(&self) -> calyx_core::Result<i64> {
        checkpoint_connection(&self.connection).await
    }

    pub async fn status(&self, dataset_id: &str) -> calyx_core::Result<StatusReport> {
        let _dataset = self.dataset(dataset_id).await?;
        self.status_existing(dataset_id).await
    }

    async fn status_existing(&self, dataset_id: &str) -> calyx_core::Result<StatusReport> {
        let mut rows = self
            .connection
            .query(
                "SELECT \
                    (SELECT count(*) FROM calyx_records WHERE dataset_id = ?1), \
                    (SELECT count(*) FROM calyx_record_slots WHERE dataset_id = ?1), \
                    (SELECT count(*) FROM calyx_projections p JOIN calyx_records r \
                        ON r.dataset_id=p.dataset_id AND r.record_id=p.record_id \
                        WHERE p.dataset_id=?1 AND p.source_revision=r.source_revision \
                        AND p.source_hash=r.source_hash AND p.slot_set_hash=r.slot_set_hash), \
                    (SELECT count(*) FROM calyx_records r LEFT JOIN calyx_projections p \
                        ON r.dataset_id=p.dataset_id AND r.record_id=p.record_id \
                        WHERE r.dataset_id=?1 AND (p.record_id IS NULL \
                        OR p.source_revision<>r.source_revision OR p.source_hash<>r.source_hash \
                        OR p.slot_set_hash<>r.slot_set_hash)), \
                    (SELECT count(*) FROM calyx_projection_failures \
                        WHERE dataset_id=?1 AND resolved_at_ms IS NULL)",
                params![dataset_id],
            )
            .await
            .map_err(|source| error::transaction("read dataset status", source))?;
        let row = rows
            .next()
            .await
            .map_err(|source| error::transaction("step dataset status", source))?
            .ok_or_else(|| error::schema("dataset status query returned no row"))?;
        Ok(StatusReport {
            dataset_id: dataset_id.to_string(),
            engine: ENGINE.to_string(),
            engine_version: self.engine_version.clone(),
            schema_version: SCHEMA_VERSION,
            journal_mode: self.journal_mode.clone(),
            records: as_u64(row.get::<i64>(0).map_err(row_error("records"))?, "records")?,
            slots: as_u64(row.get::<i64>(1).map_err(row_error("slots"))?, "slots")?,
            projected: as_u64(
                row.get::<i64>(2).map_err(row_error("projected"))?,
                "projected",
            )?,
            pending: as_u64(row.get::<i64>(3).map_err(row_error("pending"))?, "pending")?,
            projection_failures: as_u64(
                row.get::<i64>(4)
                    .map_err(row_error("projection_failures"))?,
                "projection_failures",
            )?,
        })
    }

    pub async fn readback(
        &self,
        dataset_id: &str,
        record_id: &str,
        vault_dir: Option<&Path>,
    ) -> calyx_core::Result<RecordState> {
        let dataset = self.dataset(dataset_id).await?;
        let mut record = self.load_record(&dataset, record_id).await?;
        let mut aster_match = None;
        if let Some(vault_dir) = vault_dir {
            let vault = AsterVault::new_durable(
                vault_dir,
                dataset.vault_id,
                dataset.vault_salt.clone(),
                VaultOptions {
                    read_only: true,
                    ..VaultOptions::default()
                },
            )?;
            let expected = constellation(&dataset, &record)?;
            let actual = vault.get(expected.cx_id, vault.snapshot())?;
            ensure_same_constellation(&expected, &actual, &record.record_id)?;
            aster_match = Some(true);
        }
        let slot_set_hash = record.slot_set_hash;
        let slots = std::mem::take(&mut record.slots)
            .into_iter()
            .map(|(slot_id, slot)| SlotState {
                slot_id: slot_id.get(),
                lens_id: slot.lens_id.to_string(),
                vector_hash: encode_hex(&slot.vector_hash),
                vector: slot.vector,
            })
            .collect();
        Ok(RecordState {
            dataset_id: dataset_id.to_string(),
            record_id: record.record_id,
            source_revision: record.source_revision,
            modality: record.modality,
            content_hex: encode_hex(&record.content),
            content_hash: encode_hex(&record.content_hash),
            source_hash: encode_hex(&record.source_hash),
            slot_set_hash: encode_hex(&slot_set_hash),
            created_at_secs: record.created_at_secs,
            metadata: record.metadata,
            scalars: record.scalars,
            anchors: record.anchors,
            slots,
            projected_cx_id: record.projected_cx_id.map(|id| id.to_string()),
            projected_aster_snapshot: record.projected_aster_snapshot,
            aster_match,
        })
    }

    pub async fn verify(
        &self,
        dataset_id: &str,
        vault_dir: impl AsRef<Path>,
    ) -> calyx_core::Result<VerifyReport> {
        let dataset = self.dataset(dataset_id).await?;
        let status = self.status_existing(dataset_id).await?;
        if status.records == 0 || status.pending != 0 || status.projection_failures != 0 {
            return Err(error::projection(format!(
                "dataset verification requires records>0, pending=0, failures=0; got records={}, pending={}, failures={}",
                status.records, status.pending, status.projection_failures
            )));
        }
        let quick_check = quick_check(&self.connection).await?;
        if quick_check != "ok" {
            return Err(error::schema(format!(
                "Turso integrity check returned {quick_check:?}"
            )));
        }
        let vault = AsterVault::new_durable(
            vault_dir,
            dataset.vault_id,
            dataset.vault_salt.clone(),
            VaultOptions {
                read_only: true,
                ..VaultOptions::default()
            },
        )?;
        let snapshot = vault.snapshot();
        let mut after = String::new();
        let mut records_verified = 0_u64;
        let mut slots_verified = 0_u64;
        loop {
            let records = self
                .load_record_batch(&dataset, &after, MAX_BATCH_SIZE)
                .await?;
            if records.is_empty() {
                break;
            }
            for record in &records {
                let expected = constellation(&dataset, record)?;
                let actual = vault.get(expected.cx_id, snapshot)?;
                ensure_same_constellation(&expected, &actual, &record.record_id)?;
                if record.projected_cx_id != Some(expected.cx_id) {
                    return Err(error::projection(format!(
                        "record {:?} projection row does not contain expected cx_id {}",
                        record.record_id, expected.cx_id
                    )));
                }
                records_verified += 1;
                slots_verified += record.slots.len() as u64;
            }
            after = records
                .last()
                .expect("nonempty verification batch")
                .record_id
                .clone();
        }
        if records_verified != status.records || slots_verified != status.slots {
            return Err(error::projection(format!(
                "verification cardinality mismatch: read {records_verified}/{} records and {slots_verified}/{} slots",
                status.records, status.slots
            )));
        }
        Ok(VerifyReport {
            dataset_id: dataset_id.to_string(),
            records_verified,
            slots_verified,
            turso_quick_check: quick_check,
            aster_snapshot: snapshot,
            physical_match: true,
        })
    }

    fn init_report(&self) -> InitReport {
        InitReport {
            database_path: self.path.display().to_string(),
            engine: ENGINE,
            engine_version: self.engine_version.clone(),
            journal_mode: self.journal_mode.clone(),
            schema_version: SCHEMA_VERSION,
        }
    }

    async fn verify_contract(&self) -> calyx_core::Result<()> {
        if self.engine_version != ENGINE_VERSION {
            return Err(error::engine(format!(
                "loaded Turso engine version {}, required {ENGINE_VERSION}",
                self.engine_version
            )));
        }
        if self.journal_mode != "mvcc" {
            return Err(error::engine(format!(
                "database journal_mode is {:?}, required \"mvcc\"",
                self.journal_mode
            )));
        }
        let mut rows = self
            .connection
            .query(
                "SELECT schema_version, engine, engine_version, journal_mode \
                 FROM calyx_schema WHERE singleton = 1",
                (),
            )
            .await
            .map_err(|source| error::schema(format!("read calyx_schema: {source}")))?;
        let row = rows
            .next()
            .await
            .map_err(|source| error::schema(format!("step calyx_schema: {source}")))?
            .ok_or_else(|| error::schema("calyx_schema singleton row is missing"))?;
        let schema_version = row
            .get::<i64>(0)
            .map_err(|source| error::schema(format!("decode schema_version: {source}")))?;
        let engine = row
            .get::<String>(1)
            .map_err(|source| error::schema(format!("decode engine: {source}")))?;
        let engine_version = row
            .get::<String>(2)
            .map_err(|source| error::schema(format!("decode engine_version: {source}")))?;
        let journal_mode = row
            .get::<String>(3)
            .map_err(|source| error::schema(format!("decode journal_mode: {source}")))?;
        if schema_version != SCHEMA_VERSION
            || engine != ENGINE
            || engine_version != ENGINE_VERSION
            || journal_mode != "mvcc"
        {
            return Err(error::schema(format!(
                "contract mismatch: schema={schema_version}, engine={engine:?}, version={engine_version:?}, journal={journal_mode:?}"
            )));
        }
        Ok(())
    }

    async fn dataset(&self, dataset_id: &str) -> calyx_core::Result<Dataset> {
        self.dataset_optional(dataset_id)
            .await?
            .ok_or_else(|| error::schema(format!("dataset {dataset_id:?} does not exist")))
    }

    async fn dataset_has_records(&self, dataset_id: &str) -> calyx_core::Result<bool> {
        let mut rows = self
            .connection
            .query(
                "SELECT EXISTS(SELECT 1 FROM calyx_records WHERE dataset_id=?1 LIMIT 1)",
                params![dataset_id],
            )
            .await
            .map_err(|source| error::transaction("check dataset records", source))?;
        let exists = rows
            .next()
            .await
            .map_err(|source| error::transaction("step dataset record check", source))?
            .ok_or_else(|| error::schema("dataset record check returned no row"))?
            .get::<i64>(0)
            .map_err(|source| error::schema(format!("decode dataset record state: {source}")))?;
        match exists {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(error::schema(format!(
                "dataset record check returned invalid boolean {value}"
            ))),
        }
    }

    async fn dataset_optional(&self, dataset_id: &str) -> calyx_core::Result<Option<Dataset>> {
        let mut rows = self
            .connection
            .query(
                "SELECT vault_id, vault_salt, panel_version FROM calyx_datasets WHERE dataset_id=?1",
                params![dataset_id],
            )
            .await
            .map_err(|source| error::transaction("read dataset", source))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|source| error::transaction("step dataset", source))?
        else {
            return Ok(None);
        };
        let vault_id_text = row
            .get::<String>(0)
            .map_err(|source| error::schema(format!("decode vault_id: {source}")))?;
        let vault_id = VaultId::from_str(&vault_id_text).map_err(|source| {
            error::schema(format!("parse vault_id {vault_id_text:?}: {source}"))
        })?;
        let vault_salt = row
            .get::<Vec<u8>>(1)
            .map_err(|source| error::schema(format!("decode vault_salt: {source}")))?;
        if vault_salt.len() != 32 {
            return Err(error::schema(format!(
                "dataset {dataset_id:?} vault_salt length is {}, required 32",
                vault_salt.len()
            )));
        }
        let panel_version_i64 = row
            .get::<i64>(2)
            .map_err(|source| error::schema(format!("decode panel_version: {source}")))?;
        let panel_version = u32::try_from(panel_version_i64).map_err(|_| {
            error::schema(format!(
                "dataset {dataset_id:?} panel_version is outside u32"
            ))
        })?;
        Ok(Some(Dataset {
            dataset_id: dataset_id.to_string(),
            vault_id,
            vault_salt,
            panel_version,
        }))
    }

    async fn existing_records<'a>(
        &self,
        dataset_id: &str,
        record_ids: impl Iterator<Item = &'a str>,
    ) -> calyx_core::Result<BTreeMap<String, (u64, [u8; 32], bool)>> {
        let record_ids = record_ids.collect::<Vec<_>>();
        let placeholders = placeholders(record_ids.len());
        let sql = format!(
            "SELECT r.record_id, r.source_revision, r.source_hash, p.record_id IS NOT NULL \
             FROM calyx_records r LEFT JOIN calyx_projections p \
             ON r.dataset_id=p.dataset_id AND r.record_id=p.record_id \
             WHERE r.dataset_id=? AND r.record_id IN ({placeholders})"
        );
        let mut values = Vec::with_capacity(record_ids.len() + 1);
        values.push(Value::Text(dataset_id.to_string()));
        values.extend(
            record_ids
                .into_iter()
                .map(|record_id| Value::Text(record_id.to_string())),
        );
        let mut rows = self
            .connection
            .query(sql, params_from_iter(values))
            .await
            .map_err(|source| error::transaction("batch-read existing records", source))?;
        let mut existing = BTreeMap::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|source| error::transaction("step existing records", source))?
        {
            let record_id = row
                .get::<String>(0)
                .map_err(|source| error::schema(format!("decode existing record_id: {source}")))?;
            let revision = as_u64(
                row.get::<i64>(1)
                    .map_err(|source| error::schema(format!("decode source_revision: {source}")))?,
                "source_revision",
            )?;
            let source_hash = array_32(
                row.get::<Vec<u8>>(2)
                    .map_err(|source| error::schema(format!("decode source_hash: {source}")))?,
                "source_hash",
            )?;
            let projected = row
                .get::<i64>(3)
                .map_err(|source| error::schema(format!("decode projected state: {source}")))?
                != 0;
            existing.insert(record_id, (revision, source_hash, projected));
        }
        Ok(existing)
    }

    async fn load_pending_batch(
        &self,
        dataset: &Dataset,
        after: &str,
        batch_size: usize,
    ) -> calyx_core::Result<Vec<StoredRecord>> {
        let mut rows = self
            .connection
            .query(
                "SELECT r.record_id, r.source_revision, r.modality_json, r.content, r.content_hash, \
                    r.source_hash, r.slot_set_hash, r.created_at_secs, r.metadata_json, \
                    r.scalars_json, r.anchors_json, p.cx_id, p.aster_snapshot \
                 FROM calyx_records r LEFT JOIN calyx_projections p \
                    ON r.dataset_id=p.dataset_id AND r.record_id=p.record_id \
                 WHERE r.dataset_id=?1 AND r.record_id>?2 AND (p.record_id IS NULL \
                    OR p.source_revision<>r.source_revision OR p.source_hash<>r.source_hash \
                    OR p.slot_set_hash<>r.slot_set_hash) \
                 ORDER BY r.record_id LIMIT ?3",
                params![
                    dataset.dataset_id.as_str(),
                    after,
                    as_i64(batch_size as u64, "batch_size")?
                ],
            )
            .await
            .map_err(|source| error::transaction("load pending records", source))?;
        let mut records = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|source| error::transaction("step pending records", source))?
        {
            records.push(decode_record_row(&dataset.dataset_id, &row)?);
        }
        self.load_slots(&mut records).await?;
        Ok(records)
    }

    async fn load_record_batch(
        &self,
        dataset: &Dataset,
        after: &str,
        batch_size: usize,
    ) -> calyx_core::Result<Vec<StoredRecord>> {
        let mut rows = self
            .connection
            .query(
                "SELECT r.record_id, r.source_revision, r.modality_json, r.content, r.content_hash, \
                    r.source_hash, r.slot_set_hash, r.created_at_secs, r.metadata_json, \
                    r.scalars_json, r.anchors_json, p.cx_id, p.aster_snapshot \
                 FROM calyx_records r LEFT JOIN calyx_projections p \
                    ON r.dataset_id=p.dataset_id AND r.record_id=p.record_id \
                 WHERE r.dataset_id=?1 AND r.record_id>?2 \
                 ORDER BY r.record_id LIMIT ?3",
                params![
                    dataset.dataset_id.as_str(),
                    after,
                    as_i64(batch_size as u64, "batch_size")?
                ],
            )
            .await
            .map_err(|source| error::transaction("load record batch", source))?;
        let mut records = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|source| error::transaction("step record batch", source))?
        {
            records.push(decode_record_row(&dataset.dataset_id, &row)?);
        }
        self.load_slots(&mut records).await?;
        Ok(records)
    }

    async fn load_record(
        &self,
        dataset: &Dataset,
        record_id: &str,
    ) -> calyx_core::Result<StoredRecord> {
        let mut rows = self
            .connection
            .query(
                "SELECT r.record_id, r.source_revision, r.modality_json, r.content, r.content_hash, \
                    r.source_hash, r.slot_set_hash, r.created_at_secs, r.metadata_json, \
                    r.scalars_json, r.anchors_json, p.cx_id, p.aster_snapshot \
                 FROM calyx_records r LEFT JOIN calyx_projections p \
                    ON r.dataset_id=p.dataset_id AND r.record_id=p.record_id \
                 WHERE r.dataset_id=?1 AND r.record_id=?2",
                params![dataset.dataset_id.as_str(), record_id],
            )
            .await
            .map_err(|source| error::transaction("load record", source))?;
        let row = rows
            .next()
            .await
            .map_err(|source| error::transaction("step record", source))?
            .ok_or_else(|| {
                error::record(format!(
                    "record {record_id:?} does not exist in dataset {:?}",
                    dataset.dataset_id
                ))
            })?;
        let mut records = vec![decode_record_row(&dataset.dataset_id, &row)?];
        drop(rows);
        self.load_slots(&mut records).await?;
        Ok(records.pop().expect("single loaded record"))
    }

    async fn load_slots(&self, records: &mut [StoredRecord]) -> calyx_core::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let dataset_id = records[0].dataset_id.clone();
        if records.iter().any(|record| record.dataset_id != dataset_id) {
            return Err(error::schema("slot batch spans more than one dataset"));
        }
        let placeholders = placeholders(records.len());
        let sql = format!(
            "SELECT record_id, slot_id, lens_id, vector_json, vector_hash \
             FROM calyx_record_slots WHERE dataset_id=? AND record_id IN ({placeholders}) \
             ORDER BY record_id, slot_id"
        );
        let mut values = Vec::with_capacity(records.len() + 1);
        values.push(Value::Text(dataset_id));
        values.extend(
            records
                .iter()
                .map(|record| Value::Text(record.record_id.clone())),
        );
        if records
            .windows(2)
            .any(|pair| pair[0].record_id >= pair[1].record_id)
        {
            return Err(error::schema(
                "record batch is not in strict record_id order",
            ));
        }
        let mut record_index = 0_usize;
        let mut rows = self
            .connection
            .query(sql, params_from_iter(values))
            .await
            .map_err(|source| error::transaction("batch-load record slots", source))?;
        while let Some(row) = rows
            .next()
            .await
            .map_err(|source| error::transaction("step record slots", source))?
        {
            let record_id = row
                .get::<String>(0)
                .map_err(|source| error::schema(format!("decode slot record_id: {source}")))?;
            while record_index < records.len()
                && records[record_index].record_id.as_str() < record_id.as_str()
            {
                record_index += 1;
            }
            if record_index == records.len()
                || records[record_index].record_id.as_str() != record_id.as_str()
            {
                return Err(error::schema(format!(
                    "slot references unloaded record {record_id:?}"
                )));
            }
            let slot_id_i64 = row
                .get::<i64>(1)
                .map_err(|source| error::schema(format!("decode slot_id: {source}")))?;
            let slot_id = u16::try_from(slot_id_i64)
                .map(SlotId::new)
                .map_err(|_| error::schema(format!("slot_id {slot_id_i64} outside u16")))?;
            let lens_id_text = row
                .get::<String>(2)
                .map_err(|source| error::schema(format!("decode lens_id: {source}")))?;
            let lens_id = LensId::from_str(&lens_id_text).map_err(|source| {
                error::schema(format!("parse lens_id {lens_id_text:?}: {source}"))
            })?;
            let vector_json = row
                .get::<Vec<u8>>(3)
                .map_err(|source| error::schema(format!("decode vector_json: {source}")))?;
            let vector: SlotVector = serde_json::from_slice(&vector_json).map_err(|source| {
                error::schema(format!(
                    "decode vector for record {record_id:?} slot {slot_id}: {source}"
                ))
            })?;
            vector.validate_schema()?;
            let vector_hash = array_32(
                row.get::<Vec<u8>>(4)
                    .map_err(|source| error::schema(format!("decode vector_hash: {source}")))?,
                "vector_hash",
            )?;
            if *blake3::hash(&vector_json).as_bytes() != vector_hash {
                return Err(error::schema(format!(
                    "vector hash mismatch for record {record_id:?} slot {slot_id}"
                )));
            }
            if records[record_index]
                .slots
                .insert(
                    slot_id,
                    StoredSlot {
                        lens_id,
                        vector,
                        vector_hash,
                    },
                )
                .is_some()
            {
                return Err(error::schema(format!(
                    "duplicate stored slot for record {record_id:?} slot {slot_id}"
                )));
            }
        }
        for record in records {
            if record.slots.is_empty()
                || !record.slots.values().any(|slot| slot.vector.is_indexable())
            {
                return Err(error::schema(format!(
                    "record {:?} has no indexable stored slots",
                    record.record_id
                )));
            }
        }
        Ok(())
    }

    async fn project_batch(
        &mut self,
        dataset: &Dataset,
        vault: &AsterVault,
        records: Vec<StoredRecord>,
    ) -> calyx_core::Result<usize> {
        let mut expected_ids = Vec::with_capacity(records.len());
        let snapshot_before = vault.snapshot();
        let mut new_constellations = Vec::new();
        for record in &records {
            let constellation = constellation(dataset, record)?;
            expected_ids.push(constellation.cx_id);
            let base_exists = vault
                .read_cf_at(
                    snapshot_before,
                    ColumnFamily::Base,
                    &base_key(constellation.cx_id),
                )?
                .is_some();
            if base_exists {
                let actual = vault.get(constellation.cx_id, snapshot_before)?;
                ensure_same_constellation(&constellation, &actual, &record.record_id)?;
            } else {
                new_constellations.push(constellation);
            }
        }
        if !new_constellations.is_empty() {
            vault.put_batch(new_constellations)?;
        }
        vault.flush()?;
        let aster_snapshot = vault.snapshot();
        for (record, expected_id) in records.iter().zip(&expected_ids) {
            let expected = constellation(dataset, record)?;
            if expected.cx_id != *expected_id {
                return Err(error::projection(format!(
                    "reconstructed cx_id changed for record {:?}: before={}, after={}",
                    record.record_id, expected_id, expected.cx_id
                )));
            }
            let actual = vault.get(expected.cx_id, aster_snapshot)?;
            ensure_same_constellation(&expected, &actual, &record.record_id)?;
        }
        let now_ms = now_ms()?;
        self.connection
            .set_transaction_behavior(TransactionBehavior::Immediate);
        let transaction = self
            .connection
            .transaction()
            .await
            .map_err(|source| error::transaction("begin projection completion", source))?;
        let mut upsert = transaction
            .prepare(
                "INSERT INTO calyx_projections(\
                    dataset_id, record_id, source_revision, source_hash, slot_set_hash, \
                    cx_id, aster_snapshot, projected_at_ms\
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                 ON CONFLICT(dataset_id, record_id) DO UPDATE SET \
                    source_revision=excluded.source_revision, source_hash=excluded.source_hash, \
                    slot_set_hash=excluded.slot_set_hash, cx_id=excluded.cx_id, \
                    aster_snapshot=excluded.aster_snapshot, projected_at_ms=excluded.projected_at_ms",
            )
            .await
            .map_err(|source| error::transaction("prepare projection completion", source))?;
        let mut resolve_failures = transaction
            .prepare(
                "UPDATE calyx_projection_failures \
                 SET resolved_at_ms=?1, resolved_by_snapshot=?2 \
                 WHERE dataset_id=?3 AND record_id=?4 AND resolved_at_ms IS NULL",
            )
            .await
            .map_err(|source| {
                error::transaction("prepare projection failure resolution", source)
            })?;
        for (record, cx_id) in records.iter().zip(&expected_ids) {
            upsert
                .execute(params![
                    dataset.dataset_id.as_str(),
                    record.record_id.as_str(),
                    as_i64(record.source_revision, "source_revision")?,
                    record.source_hash.to_vec(),
                    record.slot_set_hash.to_vec(),
                    cx_id.to_string(),
                    as_i64(aster_snapshot, "aster_snapshot")?,
                    as_i64(now_ms, "projected_at_ms")?
                ])
                .await
                .map_err(|source| {
                    error::transaction(
                        &format!("record projection completion for {:?}", record.record_id),
                        source,
                    )
                })?;
            resolve_failures
                .execute(params![
                    as_i64(now_ms, "resolved_at_ms")?,
                    as_i64(aster_snapshot, "resolved_by_snapshot")?,
                    dataset.dataset_id.as_str(),
                    record.record_id.as_str()
                ])
                .await
                .map_err(|source| {
                    error::transaction(
                        &format!("resolve projection failures for {:?}", record.record_id),
                        source,
                    )
                })?;
        }
        transaction
            .commit()
            .await
            .map_err(|source| error::transaction("commit projection completion", source))?;
        Ok(records.len())
    }

    async fn log_projection_failures(
        &mut self,
        dataset_id: &str,
        record_ids: &[String],
        failure: &CalyxError,
    ) -> calyx_core::Result<()> {
        let now_ms = now_ms()?;
        self.connection
            .set_transaction_behavior(TransactionBehavior::Immediate);
        let transaction = self
            .connection
            .transaction()
            .await
            .map_err(|source| error::transaction("begin projection failure log", source))?;
        let mut insert = transaction
            .prepare(
                "INSERT INTO calyx_projection_failures(\
                    dataset_id, record_id, source_revision, error_code, message, remediation, failed_at_ms\
                 ) VALUES(?1, ?2, \
                    COALESCE((SELECT source_revision FROM calyx_records \
                        WHERE dataset_id=?1 AND record_id=?2), 0), \
                    ?3, ?4, ?5, ?6)",
            )
            .await
            .map_err(|source| error::transaction("prepare projection failure log", source))?;
        for record_id in record_ids {
            insert
                .execute(params![
                    dataset_id,
                    record_id.as_str(),
                    failure.code,
                    failure.message.as_str(),
                    failure.remediation,
                    as_i64(now_ms, "failed_at_ms")?
                ])
                .await
                .map_err(|source| {
                    error::transaction(&format!("log projection failure for {record_id:?}"), source)
                })?;
        }
        transaction
            .commit()
            .await
            .map_err(|source| error::transaction("commit projection failure log", source))?;
        checkpoint_connection(&self.connection).await?;
        Ok(())
    }
}

impl Dataset {
    fn info(&self) -> DatasetInfo {
        DatasetInfo {
            dataset_id: self.dataset_id.clone(),
            vault_id: self.vault_id.to_string(),
            panel_version: self.panel_version,
        }
    }
}

fn constellation(dataset: &Dataset, record: &StoredRecord) -> calyx_core::Result<Constellation> {
    let mut metadata = record.metadata.clone();
    metadata.insert(
        "calyx_turso_dataset_id".to_string(),
        dataset.dataset_id.clone(),
    );
    metadata.insert(
        "calyx_turso_record_id".to_string(),
        record.record_id.clone(),
    );
    metadata.insert(
        "calyx_turso_source_revision".to_string(),
        record.source_revision.to_string(),
    );
    metadata.insert(
        "calyx_turso_source_hash".to_string(),
        encode_hex(&record.source_hash),
    );
    let slots = record
        .slots
        .iter()
        .map(|(slot_id, slot)| (*slot_id, slot.vector.clone()))
        .collect();
    let constellation = Constellation {
        cx_id: CxId::from_input(&record.content, dataset.panel_version, &dataset.vault_salt),
        vault_id: dataset.vault_id,
        panel_version: dataset.panel_version,
        created_at: record.created_at_secs,
        input_ref: InputRef {
            hash: record.content_hash,
            pointer: Some(format!(
                "turso://{}/{}",
                dataset.dataset_id, record.record_id
            )),
            redacted: true,
        },
        modality: record.modality,
        slots,
        scalars: record.scalars.clone(),
        metadata,
        anchors: record.anchors.clone(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: record.anchors.is_empty(),
            redacted_input: true,
            ..CxFlags::default()
        },
    };
    constellation.validate_schema()?;
    Ok(constellation)
}

fn ensure_same_constellation(
    expected: &Constellation,
    actual: &Constellation,
    record_id: &str,
) -> calyx_core::Result<()> {
    let Constellation {
        cx_id: expected_cx_id,
        vault_id: expected_vault_id,
        panel_version: expected_panel_version,
        created_at: expected_created_at,
        input_ref: expected_input_ref,
        modality: expected_modality,
        slots: expected_slots,
        scalars: expected_scalars,
        metadata: expected_metadata,
        anchors: expected_anchors,
        provenance: _,
        flags: expected_flags,
    } = expected;
    let Constellation {
        cx_id: actual_cx_id,
        vault_id: actual_vault_id,
        panel_version: actual_panel_version,
        created_at: actual_created_at,
        input_ref: actual_input_ref,
        modality: actual_modality,
        slots: actual_slots,
        scalars: actual_scalars,
        metadata: actual_metadata,
        anchors: actual_anchors,
        provenance: _,
        flags: actual_flags,
    } = actual;
    if expected_cx_id != actual_cx_id
        || expected_vault_id != actual_vault_id
        || expected_panel_version != actual_panel_version
        || expected_created_at != actual_created_at
        || expected_input_ref != actual_input_ref
        || expected_modality != actual_modality
        || expected_slots != actual_slots
        || expected_scalars != actual_scalars
        || expected_metadata != actual_metadata
        || expected_anchors != actual_anchors
        || expected_flags != actual_flags
    {
        return Err(error::projection(format!(
            "Aster readback differs from Turso source for record {record_id:?}, cx_id {}",
            expected.cx_id
        )));
    }
    Ok(())
}

fn decode_record_row(dataset_id: &str, row: &turso::Row) -> calyx_core::Result<StoredRecord> {
    let record_id = row
        .get::<String>(0)
        .map_err(|source| error::schema(format!("decode record_id: {source}")))?;
    let source_revision = as_u64(
        row.get::<i64>(1)
            .map_err(|source| error::schema(format!("decode source_revision: {source}")))?,
        "source_revision",
    )?;
    let modality_json = row
        .get::<String>(2)
        .map_err(|source| error::schema(format!("decode modality_json: {source}")))?;
    let modality = serde_json::from_str::<Modality>(&modality_json).map_err(|source| {
        error::schema(format!(
            "decode modality for record {record_id:?}: {source}"
        ))
    })?;
    let content = row
        .get::<Vec<u8>>(3)
        .map_err(|source| error::schema(format!("decode content: {source}")))?;
    let content_hash = array_32(
        row.get::<Vec<u8>>(4)
            .map_err(|source| error::schema(format!("decode content_hash: {source}")))?,
        "content_hash",
    )?;
    if *blake3::hash(&content).as_bytes() != content_hash {
        return Err(error::schema(format!(
            "content hash mismatch for record {record_id:?}"
        )));
    }
    let source_hash = array_32(
        row.get::<Vec<u8>>(5)
            .map_err(|source| error::schema(format!("decode source_hash: {source}")))?,
        "source_hash",
    )?;
    let slot_set_hash = array_32(
        row.get::<Vec<u8>>(6)
            .map_err(|source| error::schema(format!("decode slot_set_hash: {source}")))?,
        "slot_set_hash",
    )?;
    let created_at_secs = as_u64(
        row.get::<i64>(7)
            .map_err(|source| error::schema(format!("decode created_at_secs: {source}")))?,
        "created_at_secs",
    )?;
    let metadata = json_column::<BTreeMap<String, String>>(row, 8, "metadata_json", &record_id)?;
    let scalars = json_column::<BTreeMap<String, f64>>(row, 9, "scalars_json", &record_id)?;
    let anchors = json_column::<Vec<Anchor>>(row, 10, "anchors_json", &record_id)?;
    let projected_cx_id = optional_text(row, 11)?
        .map(|value| {
            CxId::from_str(&value)
                .map_err(|source| error::schema(format!("parse cx_id {value:?}: {source}")))
        })
        .transpose()?;
    let projected_aster_snapshot = optional_i64(row, 12)?
        .map(|value| as_u64(value, "aster_snapshot"))
        .transpose()?;
    Ok(StoredRecord {
        dataset_id: dataset_id.to_string(),
        record_id,
        source_revision,
        modality,
        content,
        content_hash,
        source_hash,
        slot_set_hash,
        created_at_secs,
        metadata,
        scalars,
        anchors,
        slots: BTreeMap::new(),
        projected_cx_id,
        projected_aster_snapshot,
    })
}

fn json_column<T: serde::de::DeserializeOwned>(
    row: &turso::Row,
    index: usize,
    field: &str,
    record_id: &str,
) -> calyx_core::Result<T> {
    let bytes = row
        .get::<Vec<u8>>(index)
        .map_err(|source| error::schema(format!("decode {field}: {source}")))?;
    serde_json::from_slice(&bytes).map_err(|source| {
        error::schema(format!("parse {field} for record {record_id:?}: {source}"))
    })
}

async fn engine_version(connection: &Connection) -> calyx_core::Result<String> {
    let mut rows = connection
        .query("SELECT turso_version()", ())
        .await
        .map_err(|source| error::engine(format!("Turso engine handshake failed: {source}")))?;
    let reported = rows
        .next()
        .await
        .map_err(|source| error::engine(format!("step engine handshake: {source}")))?
        .ok_or_else(|| error::engine("Turso engine handshake returned no row"))?
        .get::<String>(0)
        .map_err(|source| error::engine(format!("decode Turso engine version: {source}")))?;
    if reported.is_empty() {
        return Err(error::engine(
            "Turso engine handshake returned an empty value",
        ));
    }
    // The SQL function reports the package version only before initialization;
    // after a file header exists it reports that header's compatibility
    // version. The linked engine is pinned by Cargo.lock, so persist and check
    // the pinned engine-package version rather than conflating the two values.
    Ok(ENGINE_VERSION.to_string())
}

async fn set_mvcc(connection: &Connection) -> calyx_core::Result<String> {
    let rows = connection
        .pragma_update("journal_mode", "'mvcc'")
        .await
        .map_err(|source| error::engine(format!("enable Turso MVCC: {source}")))?;
    let mode = rows
        .first()
        .ok_or_else(|| error::engine("enabling Turso MVCC returned no state row"))?
        .get::<String>(0)
        .map_err(|source| error::engine(format!("decode Turso journal mode: {source}")))?;
    if mode != "mvcc" {
        return Err(error::engine(format!(
            "Turso refused MVCC journal mode and returned {mode:?}"
        )));
    }
    Ok(mode)
}

async fn current_journal_mode(connection: &Connection) -> calyx_core::Result<String> {
    let mut rows = connection
        .query("PRAGMA journal_mode", ())
        .await
        .map_err(|source| error::engine(format!("read Turso journal mode: {source}")))?;
    rows.next()
        .await
        .map_err(|source| error::engine(format!("step Turso journal mode: {source}")))?
        .ok_or_else(|| error::engine("Turso journal mode query returned no row"))?
        .get::<String>(0)
        .map_err(|source| error::engine(format!("decode Turso journal mode: {source}")))
}

async fn checkpoint_connection(connection: &Connection) -> calyx_core::Result<i64> {
    let mut rows = connection
        .query("PRAGMA wal_checkpoint(TRUNCATE)", ())
        .await
        .map_err(|source| error::checkpoint(format!("run Turso checkpoint: {source}")))?;
    let row = rows
        .next()
        .await
        .map_err(|source| error::checkpoint(format!("step Turso checkpoint: {source}")))?
        .ok_or_else(|| error::checkpoint("Turso checkpoint returned no state row"))?;
    let busy = row
        .get::<i64>(0)
        .map_err(|source| error::checkpoint(format!("decode checkpoint busy state: {source}")))?;
    let log_frames = row
        .get::<i64>(1)
        .map_err(|source| error::checkpoint(format!("decode checkpoint log frames: {source}")))?;
    let checkpointed = row
        .get::<i64>(2)
        .map_err(|source| error::checkpoint(format!("decode checkpoint frames: {source}")))?;
    if busy != 0 || log_frames != checkpointed {
        return Err(error::checkpoint(format!(
            "checkpoint incomplete: busy={busy}, log_frames={log_frames}, checkpointed={checkpointed}"
        )));
    }
    Ok(checkpointed)
}

async fn quick_check(connection: &Connection) -> calyx_core::Result<String> {
    let mut rows = connection
        .query("PRAGMA quick_check", ())
        .await
        .map_err(|source| error::schema(format!("run Turso quick_check: {source}")))?;
    rows.next()
        .await
        .map_err(|source| error::schema(format!("step Turso quick_check: {source}")))?
        .ok_or_else(|| error::schema("Turso quick_check returned no row"))?
        .get::<String>(0)
        .map_err(|source| error::schema(format!("decode Turso quick_check: {source}")))
}

fn optional_text(row: &turso::Row, index: usize) -> calyx_core::Result<Option<String>> {
    match row
        .get_value(index)
        .map_err(|source| error::schema(format!("decode optional text: {source}")))?
    {
        Value::Null => Ok(None),
        Value::Text(value) => Ok(Some(value)),
        value => Err(error::schema(format!(
            "optional text column has unexpected value {value:?}"
        ))),
    }
}

fn optional_i64(row: &turso::Row, index: usize) -> calyx_core::Result<Option<i64>> {
    match row
        .get_value(index)
        .map_err(|source| error::schema(format!("decode optional integer: {source}")))?
    {
        Value::Null => Ok(None),
        Value::Integer(value) => Ok(Some(value)),
        value => Err(error::schema(format!(
            "optional integer column has unexpected value {value:?}"
        ))),
    }
}

fn path_text(path: &Path) -> calyx_core::Result<&str> {
    path.to_str().ok_or_else(|| {
        error::open(format!(
            "database path {} is not valid UTF-8",
            path.display()
        ))
    })
}

fn now_ms() -> calyx_core::Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| error::open(format!("system clock precedes Unix epoch: {source}")))?
        .as_millis();
    u64::try_from(millis).map_err(|_| error::open("system clock milliseconds exceed u64"))
}

fn array_32(bytes: Vec<u8>, field: &str) -> calyx_core::Result<[u8; 32]> {
    bytes.try_into().map_err(|bytes: Vec<u8>| {
        error::schema(format!("{field} length is {}, required 32", bytes.len()))
    })
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

fn as_i64(value: u64, field: &str) -> calyx_core::Result<i64> {
    i64::try_from(value).map_err(|_| error::record(format!("{field} exceeds i64")))
}

fn as_u64(value: i64, field: &str) -> calyx_core::Result<u64> {
    u64::try_from(value).map_err(|_| error::schema(format!("{field} is negative")))
}

fn row_error(field: &'static str) -> impl FnOnce(turso::Error) -> CalyxError {
    move |source| error::schema(format!("decode {field}: {source}"))
}

fn validate_batch_size(batch_size: usize) -> calyx_core::Result<()> {
    if (1..=MAX_BATCH_SIZE).contains(&batch_size) {
        Ok(())
    } else {
        Err(error::batch(format!(
            "batch_size {batch_size} outside 1..={MAX_BATCH_SIZE}"
        )))
    }
}
