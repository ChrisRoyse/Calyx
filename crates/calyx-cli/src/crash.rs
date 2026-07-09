use crate::cf_read::hex_bytes;
use crate::error::{CliError, CliResult};
use calyx_aster::cf::{ColumnFamily, base_key, ledger_key, slot_key};
use calyx_aster::manifest::{ImmutableRef, ManifestStore, VaultManifest};
use calyx_aster::sst::write_sst;
use calyx_aster::vault::encode::{
    WriteRow, decode_write_batch, encode_constellation_base, encode_slot_vector, encode_write_batch,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::{Wal, WalOptions, replay_dir};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
    VaultStore,
};
use calyx_ledger::{ActorId, EntryKind, LedgerEntry, PayloadBuilder, RedactionPolicy, SubjectId};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::process;
use std::thread;
use std::time::Duration;

const PRECOMMITTED_RECORDS: u8 = 2;
const TARGET_RECORD: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashPoint {
    BeforeWalFsync,
    AfterWalBeforeCommit,
    AfterCommitBeforeManifest,
}

impl CrashPoint {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "before-wal-fsync" => Ok(Self::BeforeWalFsync),
            "after-wal-before-commit" => Ok(Self::AfterWalBeforeCommit),
            "after-commit-before-manifest" => Ok(Self::AfterCommitBeforeManifest),
            _ => Err(format!("unsupported crash point: {value}")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::BeforeWalFsync => "before-wal-fsync",
            Self::AfterWalBeforeCommit => "after-wal-before-commit",
            Self::AfterCommitBeforeManifest => "after-commit-before-manifest",
        }
    }

    fn expected_recovered(self) -> u8 {
        match self {
            Self::BeforeWalFsync => PRECOMMITTED_RECORDS,
            Self::AfterWalBeforeCommit | Self::AfterCommitBeforeManifest => TARGET_RECORD,
        }
    }
}

pub fn crash_drill(
    vault: &Path,
    point: CrashPoint,
    pause_ms: Option<u64>,
) -> crate::error::CliResult {
    ensure_empty_vault(vault)?;
    let vault_id = crash_vault_id()?;
    let writer = AsterVault::new_durable(
        vault,
        vault_id,
        b"calyx-crash-drill-salt",
        VaultOptions::default(),
    )?;
    for index in 1..=PRECOMMITTED_RECORDS {
        writer.put(synthetic_constellation(vault_id, index))?;
    }
    writer.flush()?;
    drop(writer);

    let target = synthetic_constellation(vault_id, TARGET_RECORD);
    let rows = constellation_rows(vault, vault_id, &target)?;
    match point {
        CrashPoint::BeforeWalFsync => append_torn_header(vault, TARGET_RECORD as u64)?,
        CrashPoint::AfterWalBeforeCommit => {
            append_wal_batch(vault, TARGET_RECORD as u64, &rows)?;
        }
        CrashPoint::AfterCommitBeforeManifest => {
            append_wal_batch(vault, TARGET_RECORD as u64, &rows)?;
            write_batch_ssts(vault, TARGET_RECORD as u64, &rows, "CRASH_ROW")?;
        }
    }

    println!(
        "CRASH_DRILL\tPOINT\t{}\tPID\t{}\tPRECOMMITTED\t{}\tTARGET\t{}\tEXPECTED_RECOVERED\t{}\tVAULT\t{}",
        point.as_str(),
        process::id(),
        PRECOMMITTED_RECORDS,
        TARGET_RECORD,
        point.expected_recovered(),
        vault.display()
    );
    crash_exit(pause_ms);
}

pub fn recover(vault: &Path) -> crate::error::CliResult {
    let store = ManifestStore::open(vault);
    let durable_seq = if vault.join("CURRENT").exists() {
        store.load_current()?.durable_seq
    } else {
        0
    };
    let replay = replay_dir(vault.join("wal"))?;
    if let Some(torn) = &replay.torn_tail {
        println!(
            "RECOVER_TORN\tCODE\t{}\tFILE\t{}\tOFFSET\t{}\tMESSAGE\t{}",
            torn.code,
            torn.segment_path.display(),
            torn.offset,
            torn.message
        );
    }

    let mut last_recovered = durable_seq;
    let mut rows_written = 0;
    for record in replay.records {
        if record.seq <= durable_seq {
            continue;
        }
        let rows = decode_write_batch(&record.payload)?;
        println!(
            "RECOVER_RECORD\tSEQ\t{}\tROWS\t{}\tPAYLOAD\t{}",
            record.seq,
            rows.len(),
            hex_bytes(&record.payload)
        );
        rows_written += write_batch_ssts(vault, record.seq, &rows, "RECOVER_ROW")?;
        last_recovered = record.seq;
    }

    if last_recovered > durable_seq {
        let write = write_manifest(vault, last_recovered)?;
        println!(
            "RECOVER_MANIFEST\tSEQ\t{}\tCURRENT\t{}\tMANIFEST\t{}",
            last_recovered,
            write.current_path.display(),
            write.manifest_path.display()
        );
    }
    println!(
        "RECOVER_DONE\tLAST_RECOVERED_SEQ\t{}\tDURABLE_BEFORE\t{}\tROWS_WRITTEN\t{}\tVAULT\t{}",
        last_recovered,
        durable_seq,
        rows_written,
        vault.display()
    );
    Ok(())
}

pub fn open_check(vault: &Path, index: u8) -> crate::error::CliResult {
    let vault_id = crash_vault_id()?;
    let expected = synthetic_constellation(vault_id, index);
    let id = expected.cx_id;
    let opened = AsterVault::open(
        vault,
        vault_id,
        b"calyx-crash-drill-salt",
        VaultOptions::default(),
    )?;
    let snapshot = opened.snapshot();
    let got = opened.get(id, snapshot)?;
    let mut expected = expected;
    expected.provenance = got.provenance.clone();
    if got != expected {
        return Err(CliError::runtime(format!(
            "open-check mismatch for index {index}"
        )));
    }
    if got.provenance.hash == [0; 32] {
        return Err(CliError::runtime(format!(
            "open-check found zero ledger hash for index {index}"
        )));
    }
    println!(
        "OPEN_CHECK\tINDEX\t{}\tID\t{}\tSNAPSHOT\t{}\tVAULT\t{}",
        index,
        id,
        snapshot,
        vault.display()
    );
    Ok(())
}

fn ensure_empty_vault(vault: &Path) -> CliResult {
    if !vault.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(vault)?;
    if entries.next().transpose()?.is_some() {
        return Err(CliError::runtime(format!(
            "crash-drill vault must be absent or empty: {}",
            vault.display()
        )));
    }
    Ok(())
}

fn append_torn_header(vault: &Path, seq: u64) -> CliResult {
    let segment = vault.join("wal").join("00000000000000000000.wal");
    let mut file = OpenOptions::new().append(true).open(&segment)?;
    let start = file.metadata()?.len();
    file.write_all(b"CXW1")?;
    file.write_all(&seq.to_le_bytes())?;
    file.write_all(&64_u32.to_le_bytes())?;
    println!(
        "CRASH_TORN_WAL\tSEQ\t{}\tFILE\t{}\tSTART\t{}\tBYTES_WRITTEN\t16",
        seq,
        segment.display(),
        start
    );
    Ok(())
}

fn append_wal_batch(vault: &Path, expected_seq: u64, rows: &[WriteRow]) -> CliResult {
    let mut wal = Wal::open(vault.join("wal"), WalOptions::default())?;
    let payload = encode_write_batch(rows)?;
    let ack = wal.append(&payload)?;
    if ack.seq != expected_seq {
        return Err(CliError::runtime(format!(
            "unexpected WAL seq {}, expected {expected_seq}",
            ack.seq
        )));
    }
    println!(
        "CRASH_WAL_APPEND\tSEQ\t{}\tFILE\t{}\tSTART\t{}\tEND\t{}\tPAYLOAD\t{}",
        ack.seq,
        ack.segment_path.display(),
        ack.start_offset,
        ack.end_offset,
        hex_bytes(&payload)
    );
    Ok(())
}

fn write_batch_ssts(vault: &Path, seq: u64, rows: &[WriteRow], tag: &str) -> CliResult<usize> {
    for (index, row) in rows.iter().enumerate() {
        let dir = vault.join("cf").join(row.cf.name());
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{seq:020}-{index:04}.sst"));
        let summary = write_sst(&path, [(row.key.as_slice(), row.value.as_slice())])?;
        println!(
            "{}\tSEQ\t{}\tCF\t{}\tFILE\t{}\tKEY\t{}\tVALUE\t{}\tBYTES\t{}",
            tag,
            seq,
            row.cf.name(),
            summary.path.display(),
            hex_bytes(&row.key),
            hex_bytes(&row.value),
            summary.bytes
        );
    }
    Ok(rows.len())
}

fn write_manifest(vault: &Path, seq: u64) -> CliResult<calyx_aster::manifest::ManifestWrite> {
    let (panel_ref, codebook_refs) = ensure_manifest_assets(vault)?;
    let manifest = VaultManifest::new(seq, seq, panel_ref, codebook_refs)?;
    Ok(ManifestStore::open(vault).write_current(&manifest)?)
}

fn ensure_manifest_assets(vault: &Path) -> CliResult<(ImmutableRef, Vec<ImmutableRef>)> {
    let panel_bytes = generated_asset_bytes("panel", "no-active-panel");
    let panel_hash = blake3::hash(&panel_bytes).to_hex().to_string();
    let panel_logical = format!("panel/generated-no-active-panel-{}.json", &panel_hash[..16]);
    let panel_path = vault.join(&panel_logical);
    let codebook_bytes = generated_asset_bytes("codebook", "no-active-codebook");
    let codebook_hash = blake3::hash(&codebook_bytes).to_hex().to_string();
    let codebook_logical = format!(
        "codebooks/generated-no-active-codebook-{}.json",
        &codebook_hash[..16]
    );
    let codebook_path = vault.join(&codebook_logical);
    write_asset(&panel_path, &panel_bytes)?;
    write_asset(&codebook_path, &codebook_bytes)?;
    Ok((
        ImmutableRef::from_bytes(panel_logical, &panel_bytes)?,
        vec![ImmutableRef::from_bytes(codebook_logical, &codebook_bytes)?],
    ))
}

fn generated_asset_bytes(kind: &str, status: &str) -> Vec<u8> {
    json!({
        "kind": "calyx_manifest_generated_asset_v1",
        "asset_kind": kind,
        "status": status
    })
    .to_string()
    .into_bytes()
}

fn write_asset(path: &Path, bytes: &[u8]) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn constellation_rows(
    vault: &Path,
    vault_id: VaultId,
    cx: &Constellation,
) -> CliResult<Vec<WriteRow>> {
    let tip = ledger_tip(vault, vault_id)?;
    let ledger_entry = ledger_entry_after_tip(cx, tip);
    let ledger_ref = LedgerRef {
        seq: ledger_entry.seq,
        hash: ledger_entry.entry_hash,
    };
    let mut cx = cx.clone();
    cx.provenance = ledger_ref;
    let mut rows = vec![WriteRow {
        cf: ColumnFamily::Base,
        key: base_key(cx.cx_id),
        value: encode_constellation_base(&cx)?,
    }];
    for (slot, vector) in &cx.slots {
        rows.push(WriteRow {
            cf: ColumnFamily::slot(*slot),
            key: slot_key(cx.cx_id),
            value: encode_slot_vector(vector)?,
        });
    }
    rows.push(WriteRow {
        cf: ColumnFamily::Ledger,
        key: ledger_key(ledger_entry.seq),
        value: calyx_ledger::encode(&ledger_entry),
    });
    Ok(rows)
}

#[derive(Clone, Copy, Debug)]
struct LedgerTip {
    next_seq: u64,
    prev_hash: [u8; 32],
    last_ts: u64,
}

fn ledger_tip(vault: &Path, vault_id: VaultId) -> CliResult<LedgerTip> {
    let reader = AsterVault::open(
        vault,
        vault_id,
        b"calyx-crash-drill-salt",
        VaultOptions {
            read_only: true,
            ..VaultOptions::default()
        },
    )?;
    let mut rows = reader.scan_cf_at(reader.snapshot(), ColumnFamily::Ledger)?;
    rows.sort_by_key(|(key, _)| parse_ledger_key(key).unwrap_or(u64::MAX));
    let Some((key, bytes)) = rows.last() else {
        return Ok(LedgerTip {
            next_seq: 0,
            prev_hash: [0; 32],
            last_ts: 0,
        });
    };
    let key_seq = parse_ledger_key(key)?;
    let entry = calyx_ledger::decode(bytes)?;
    if entry.seq != key_seq {
        return Err(CliError::runtime(format!(
            "ledger key seq {key_seq} != encoded seq {}",
            entry.seq
        )));
    }
    Ok(LedgerTip {
        next_seq: entry.seq.saturating_add(1),
        prev_hash: entry.entry_hash,
        last_ts: entry.ts,
    })
}

fn parse_ledger_key(key: &[u8]) -> CliResult<u64> {
    let bytes: [u8; 8] = key
        .try_into()
        .map_err(|_| CliError::runtime(format!("ledger key length {} != 8", key.len())))?;
    Ok(u64::from_be_bytes(bytes))
}

fn ledger_entry_after_tip(cx: &Constellation, tip: LedgerTip) -> LedgerEntry {
    LedgerEntry::new(
        tip.next_seq,
        tip.prev_hash,
        EntryKind::Ingest,
        SubjectId::Cx(cx.cx_id),
        crash_ledger_payload(cx),
        ActorId::Service("calyx-aster".to_string()),
        tip.last_ts.saturating_add(1),
    )
}

fn crash_ledger_payload(cx: &Constellation) -> Vec<u8> {
    let mut payload = PayloadBuilder::default();
    payload
        .insert_str("cx_id", cx.cx_id.to_string())
        .insert_str("input_hash", hex_bytes(&cx.input_ref.hash))
        .insert_value(
            "input_ref",
            json!({
                "hash": cx.input_ref.hash,
                "redacted": true,
            }),
        )
        .insert_u64("ts", cx.created_at);
    RedactionPolicy::default().apply_to_payload(&payload)
}

fn synthetic_constellation(vault_id: VaultId, index: u8) -> Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 3,
            data: vec![index as f32, index as f32 + 0.25, index as f32 + 0.5],
        },
    );
    Constellation {
        cx_id: CxId::from_bytes([index; 16]),
        vault_id,
        panel_version: 10,
        created_at: 1780822800 + index as u64,
        input_ref: InputRef {
            hash: [index; 32],
            pointer: Some(format!("synthetic://calyx-crash-drill/{index}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: index as u64,
            hash: [0x40 + index; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn crash_vault_id() -> CliResult<VaultId> {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .map_err(|error| CliError::runtime(format!("crash vault id parse: {error}")))
}

fn crash_exit(pause_ms: Option<u64>) -> ! {
    let _ = io::stdout().flush();
    if let Some(ms) = pause_ms {
        thread::sleep(Duration::from_millis(ms));
    }
    process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_points_parse_expected_names() {
        assert_eq!(
            CrashPoint::parse("before-wal-fsync").unwrap(),
            CrashPoint::BeforeWalFsync
        );
        assert!(CrashPoint::parse("missing").is_err());
    }
}
