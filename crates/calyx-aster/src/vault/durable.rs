use super::encode::{WriteRow, decode_write_batch, encode_write_batch};
use crate::cf::ColumnFamily;
use crate::compaction::TieringPolicy;
use crate::dedup::DedupPolicy;
use crate::manifest::{ImmutableRef, ManifestStore, VaultManifest, recover_vault};
use crate::sst::{SstReader, write_sst};
use crate::wal::{GroupCommitBatcher, WalOptions, replay_dir};
use calyx_core::{CalyxError, Panel, Result, SlotId, SystemClock, TemporalPolicy};
use calyx_ledger::CheckpointConfig;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct VaultOptions {
    pub wal_options: WalOptions,
    pub memtable_byte_cap: usize,
    pub tiering_policy: Option<TieringPolicy>,
    pub ledger_checkpoint: Option<CheckpointConfig>,
    pub temporal_policy: Option<TemporalPolicy>,
    pub dedup_policy: Option<DedupPolicy>,
    pub panel: Option<Panel>,
}

impl Default for VaultOptions {
    fn default() -> Self {
        Self {
            wal_options: WalOptions::default(),
            memtable_byte_cap: 0,
            tiering_policy: None,
            ledger_checkpoint: Some(CheckpointConfig::default()),
            temporal_policy: Some(TemporalPolicy::default()),
            dedup_policy: Some(DedupPolicy::default()),
            panel: None,
        }
    }
}

#[derive(Debug)]
pub(super) struct DurableVault {
    root: PathBuf,
    batcher: GroupCommitBatcher,
    tiering_policy: Option<TieringPolicy>,
    ledger_checkpoint: Option<CheckpointConfig>,
    temporal_policy: Option<TemporalPolicy>,
    dedup_policy: Option<DedupPolicy>,
    panel: Option<Panel>,
    pending_checkpoint: Mutex<Vec<(u64, Vec<WriteRow>)>>,
    #[cfg(test)]
    fail_next_wal_append: Arc<AtomicBool>,
}

pub(super) struct RecoveredBatch {
    pub seq: u64,
    pub rows: Vec<WriteRow>,
}

pub(super) struct RecoveredBatches {
    pub batches: Vec<RecoveredBatch>,
    pub last_recovered_seq: u64,
    pub torn_tail: Option<crate::wal::TornTail>,
    pub temporal_policy: Option<TemporalPolicy>,
    pub dedup_policy: Option<DedupPolicy>,
}

impl DurableVault {
    pub(super) fn validate_options(options: &VaultOptions) -> Result<()> {
        if let Some(policy) = &options.temporal_policy {
            policy.validate()?;
        }
        if let Some(policy) = &options.dedup_policy {
            validate_dedup_policy(policy, options.panel.as_ref())?;
        }
        Ok(())
    }

    pub(super) fn open(root: impl AsRef<Path>, options: &VaultOptions) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        Self::validate_options(options)?;
        fs::create_dir_all(root.join("cf"))
            .map_err(|error| storage_error("create durable CF root", error))?;
        if let Some(policy) = &options.tiering_policy {
            for tier_root in policy.tier_roots() {
                fs::create_dir_all(tier_root.join("cf"))
                    .map_err(|error| storage_error("create tiered durable CF root", error))?;
            }
        }
        let wal = crate::wal::Wal::open(root.join("wal"), options.wal_options)?;
        let batcher = GroupCommitBatcher::new(
            wal,
            options.wal_options.group_commit_window,
            Arc::new(SystemClock),
        )?;
        Ok(Self {
            root,
            batcher,
            tiering_policy: options.tiering_policy.clone(),
            ledger_checkpoint: options.ledger_checkpoint.clone(),
            temporal_policy: options.temporal_policy,
            dedup_policy: options.dedup_policy.clone(),
            panel: options.panel.clone(),
            pending_checkpoint: Mutex::new(Vec::new()),
            #[cfg(test)]
            fail_next_wal_append: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(super) fn recover_batches(
        root: impl AsRef<Path>,
        options: &VaultOptions,
    ) -> Result<RecoveredBatches> {
        Self::validate_options(options)?;
        let root = root.as_ref();
        if root.join("CURRENT").exists() {
            let recovery = recover_vault(root)?;
            if let Some(policy) = &recovery.manifest.dedup_policy {
                validate_dedup_policy(policy, options.panel.as_ref())?;
            }
            let mut batches = read_manifested_batches(
                root,
                options.tiering_policy.as_ref(),
                recovery.manifest.durable_seq,
            )?;
            for record in recovery.wal_records {
                batches.push(RecoveredBatch {
                    seq: record.seq,
                    rows: decode_write_batch(&record.payload)?,
                });
            }
            return Ok(RecoveredBatches {
                batches,
                last_recovered_seq: recovery.last_recovered_seq,
                torn_tail: recovery.torn_tail,
                temporal_policy: recovery.manifest.temporal_policy,
                dedup_policy: recovery.manifest.dedup_policy,
            });
        }

        let replay = replay_dir(root.join("wal"))?;
        let last_recovered_seq = replay.records.last().map_or(0, |record| record.seq);
        let batches = replay
            .records
            .iter()
            .map(|record| {
                Ok(RecoveredBatch {
                    seq: record.seq,
                    rows: decode_write_batch(&record.payload)?,
                })
            })
            .collect::<Result<_>>()?;
        Ok(RecoveredBatches {
            batches,
            last_recovered_seq,
            torn_tail: replay.torn_tail,
            temporal_policy: options.temporal_policy,
            dedup_policy: options.dedup_policy.clone(),
        })
    }

    pub(super) fn append_batch(&self, rows: &[WriteRow]) -> Result<u64> {
        #[cfg(test)]
        if self.fail_next_wal_append.swap(false, Ordering::SeqCst) {
            return Err(CalyxError::disk_pressure("injected WAL append failure"));
        }
        let payload = encode_write_batch(rows)?;
        let ack = self.batcher.submit(payload)?;
        Ok(ack.seq)
    }

    pub(super) fn durable_tip_seq(&self) -> Result<u64> {
        self.batcher.tip_seq()
    }

    #[cfg(test)]
    pub(super) fn fail_next_wal_append(&self) {
        self.fail_next_wal_append.store(true, Ordering::SeqCst);
    }

    pub(super) fn checkpoint_batch(&self, seq: u64, rows: &[WriteRow]) -> Result<()> {
        self.write_rows(seq, rows)?;
        self.write_manifest(seq)
    }

    pub(super) fn stage_checkpoint_batch(&self, seq: u64, rows: &[WriteRow]) -> Result<()> {
        self.pending_checkpoint
            .lock()
            .map_err(|_| CalyxError::disk_pressure("checkpoint staging lock poisoned"))?
            .push((seq, rows.to_vec()));
        Ok(())
    }

    pub(super) fn flush(&self) -> Result<()> {
        self.batcher.flush_sync()?;
        self.flush_pending_checkpoints()
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn recurrence_lock_path(&self) -> PathBuf {
        self.root.join("locks").join("recurrence.write.lock")
    }

    pub(super) fn commit_lock_path(&self) -> PathBuf {
        self.root.join("locks").join("durable.commit.lock")
    }

    pub(super) fn recover_current_batches(&self) -> Result<RecoveredBatches> {
        let options = VaultOptions {
            tiering_policy: self.tiering_policy.clone(),
            ledger_checkpoint: self.ledger_checkpoint.clone(),
            temporal_policy: self.temporal_policy,
            dedup_policy: self.dedup_policy.clone(),
            panel: self.panel.clone(),
            ..VaultOptions::default()
        };
        Self::recover_batches(&self.root, &options)
    }

    pub(super) fn ledger_checkpoint(&self) -> Option<CheckpointConfig> {
        self.ledger_checkpoint.clone()
    }

    pub(super) fn tiering_policy(&self) -> Option<&TieringPolicy> {
        self.tiering_policy.as_ref()
    }

    pub(super) fn compaction_output_path(&self, cf: ColumnFamily, seq: u64) -> PathBuf {
        self.cf_dir(cf).join(format!("compacted-{seq:020}.sst"))
    }

    fn write_rows(&self, seq: u64, rows: &[WriteRow]) -> Result<()> {
        let mut by_cf = Vec::<(ColumnFamily, Vec<(usize, &WriteRow)>)>::new();
        for (index, row) in rows.iter().enumerate() {
            if let Some((_, group)) = by_cf.iter_mut().find(|(cf, _)| *cf == row.cf) {
                group.push((index, row));
            } else {
                by_cf.push((row.cf, vec![(index, row)]));
            }
        }
        by_cf.sort_by_key(|(cf, _)| cf.name());
        for (cf, mut rows) in by_cf {
            rows.sort_by(|(_, left), (_, right)| left.key.cmp(&right.key));
            let first_index = rows.first().map_or(0, |(index, _)| *index);
            let dir = self.cf_dir(cf);
            fs::create_dir_all(&dir).map_err(|error| storage_error("create CF dir", error))?;
            let path = dir.join(format!("{seq:020}-{first_index:04}.sst"));
            let entries = rows
                .iter()
                .map(|(_, row)| (row.key.as_slice(), row.value.as_slice()));
            write_sst(&path, entries)?;
        }
        Ok(())
    }

    fn flush_pending_checkpoints(&self) -> Result<()> {
        let batches = self
            .pending_checkpoint
            .lock()
            .map_err(|_| CalyxError::disk_pressure("checkpoint staging lock poisoned"))?
            .clone();
        if batches.is_empty() {
            return Ok(());
        }
        for (seq, rows) in &batches {
            self.write_rows(*seq, rows)?;
        }
        let last_seq = batches.last().map_or(0, |(seq, _)| *seq);
        self.write_manifest(last_seq)?;
        let mut pending = self
            .pending_checkpoint
            .lock()
            .map_err(|_| CalyxError::disk_pressure("checkpoint staging lock poisoned"))?;
        pending.retain(|(seq, _)| *seq > last_seq);
        Ok(())
    }

    fn cf_dir(&self, cf: ColumnFamily) -> PathBuf {
        self.tiering_policy.as_ref().map_or_else(
            || self.root.join("cf").join(cf.name()),
            |policy| policy.place_current_cf(cf).absolute_dir(),
        )
    }

    fn write_manifest(&self, seq: u64) -> Result<()> {
        let (panel_ref, codebook_refs) = ensure_manifest_assets(&self.root)?;
        let manifest = VaultManifest::new_with_policies(
            seq,
            seq,
            panel_ref,
            codebook_refs,
            self.temporal_policy,
            self.dedup_policy.clone(),
        )?;
        ManifestStore::open(&self.root).write_current(&manifest)?;
        Ok(())
    }
}

fn validate_dedup_policy(policy: &DedupPolicy, panel: Option<&Panel>) -> Result<()> {
    if let Some(panel) = panel {
        policy.validate(panel)
    } else {
        policy.validate_manifest()
    }
}

fn read_manifested_batches(
    root: &Path,
    tiering_policy: Option<&TieringPolicy>,
    durable_seq: u64,
) -> Result<Vec<RecoveredBatch>> {
    let mut by_seq = BTreeMap::<u64, Vec<(usize, WriteRow)>>::new();
    if durable_seq == 0 {
        return Ok(Vec::new());
    }
    for cf_root in tiered_cf_roots(root, tiering_policy) {
        if !cf_root.exists() {
            continue;
        }
        for entry in fs::read_dir(&cf_root).map_err(|error| storage_error("read CF root", error))? {
            let cf_dir = entry.map_err(|error| storage_error("read CF entry", error))?;
            if !cf_dir
                .file_type()
                .map_err(|error| storage_error("stat CF entry", error))?
                .is_dir()
            {
                continue;
            }
            let cf_name = cf_dir.file_name().to_string_lossy().to_string();
            let cf = parse_cf_name(&cf_name)?;
            for file in
                fs::read_dir(cf_dir.path()).map_err(|error| storage_error("read CF dir", error))?
            {
                let path = file
                    .map_err(|error| storage_error("read SST entry", error))?
                    .path();
                if path.extension().and_then(|value| value.to_str()) != Some("sst") {
                    continue;
                }
                let Some((seq, index)) = durable_sst_identity(&path) else {
                    continue;
                };
                if seq > durable_seq {
                    continue;
                }
                let reader = SstReader::open(&path)?;
                for (row_offset, row) in reader.iter()?.into_iter().enumerate() {
                    by_seq.entry(seq).or_default().push((
                        index + row_offset,
                        WriteRow {
                            cf,
                            key: row.key,
                            value: row.value,
                        },
                    ));
                }
            }
        }
    }

    Ok(by_seq
        .into_iter()
        .map(|(seq, mut rows)| {
            rows.sort_by_key(|(index, _)| *index);
            RecoveredBatch {
                seq,
                rows: rows.into_iter().map(|(_, row)| row).collect(),
            }
        })
        .collect())
}

fn tiered_cf_roots(root: &Path, tiering_policy: Option<&TieringPolicy>) -> Vec<PathBuf> {
    let mut roots = vec![root.join("cf")];
    if let Some(policy) = tiering_policy {
        for tier_root in policy.tier_roots() {
            let cf_root = tier_root.join("cf");
            if !roots.contains(&cf_root) {
                roots.push(cf_root);
            }
        }
    }
    roots
}

fn durable_sst_identity(path: &Path) -> Option<(u64, usize)> {
    let stem = path.file_stem()?.to_str()?;
    if let Some(seq) = stem.strip_prefix("compacted-") {
        return Some((seq.parse().ok()?, 0));
    }
    let (seq, index) = stem.split_once('-')?;
    Some((seq.parse().ok()?, index.parse().ok()?))
}

fn parse_cf_name(value: &str) -> Result<ColumnFamily> {
    match value {
        "base" => Ok(ColumnFamily::Base),
        "anchors" => Ok(ColumnFamily::Anchors),
        "ledger" => Ok(ColumnFamily::Ledger),
        "recurrence" => Ok(ColumnFamily::Recurrence),
        "graph" => Ok(ColumnFamily::Graph),
        "online" => Ok(ColumnFamily::Online),
        "scalars" => Ok(ColumnFamily::Scalars),
        "xterm" => Ok(ColumnFamily::XTerm),
        "temporal_xterm" => Ok(ColumnFamily::TemporalXTerm),
        "assay" => Ok(ColumnFamily::Assay),
        "anneal_rollback" => Ok(ColumnFamily::AnnealRollback),
        "anneal_health" => Ok(ColumnFamily::AnnealHealth),
        "anneal_checksums" => Ok(ColumnFamily::AnnealChecksums),
        "anneal_mistakes" => Ok(ColumnFamily::AnnealMistakes),
        "anneal_replay" => Ok(ColumnFamily::AnnealReplay),
        "anneal_heads" => Ok(ColumnFamily::AnnealHeads),
        _ if value.starts_with("slot_") => parse_slot_cf(value),
        _ => Err(CalyxError::aster_corrupt_shard(format!(
            "unknown durable CF directory {value}"
        ))),
    }
}

fn parse_slot_cf(value: &str) -> Result<ColumnFamily> {
    let raw = value.ends_with(".raw");
    let slot_text = value.trim_start_matches("slot_").trim_end_matches(".raw");
    let slot = slot_text.parse::<u16>().map_err(|error| {
        CalyxError::aster_corrupt_shard(format!("invalid slot CF directory {value}: {error}"))
    })?;
    if raw {
        Ok(ColumnFamily::slot_raw(SlotId::new(slot)))
    } else {
        Ok(ColumnFamily::slot(SlotId::new(slot)))
    }
}

fn ensure_manifest_assets(root: &Path) -> Result<(ImmutableRef, Vec<ImmutableRef>)> {
    let panel_path = root.join("panel/current.bin");
    let codebook_path = root.join("codebooks/default.bin");
    let panel_bytes = b"calyx-stage1-panel";
    let codebook_bytes = b"calyx-stage1-codebook";
    write_asset(&panel_path, panel_bytes)?;
    write_asset(&codebook_path, codebook_bytes)?;
    Ok((
        ImmutableRef::from_bytes("panel/current.bin", panel_bytes)?,
        vec![ImmutableRef::from_bytes(
            "codebooks/default.bin",
            codebook_bytes,
        )?],
    ))
}

fn write_asset(path: &Path, bytes: &[u8]) -> Result<()> {
    match fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "manifest immutable asset {} hash mismatch",
                path.display()
            )));
        }
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            return Err(storage_error("read manifest asset", error));
        }
        Err(_) => {}
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| storage_error("create manifest asset dir", error))?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("manifest-asset");
    let tmp = path.with_file_name(format!(
        ".{file_name}.{:?}.tmp",
        std::thread::current().id()
    ));
    {
        let mut file =
            File::create(&tmp).map_err(|error| storage_error("create manifest asset", error))?;
        file.write_all(bytes)
            .map_err(|error| storage_error("write manifest asset", error))?;
        file.sync_all()
            .map_err(|error| storage_error("fsync manifest asset", error))?;
    }
    fs::rename(&tmp, path).map_err(|error| storage_error("install manifest asset", error))?;
    Ok(())
}

fn storage_error(context: &str, error: io::Error) -> CalyxError {
    CalyxError::disk_pressure(format!("{context}: {error}"))
}
