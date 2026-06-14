use crate::cf_read::hex_bytes;
use calyx_aster::cf::{ColumnFamily, base_key, ledger_key, slot_key};
use calyx_aster::manifest::{ImmutableRef, ManifestStore, VaultManifest};
use calyx_aster::sst::write_sst;
use calyx_aster::vault::encode::{
    WriteRow, decode_write_batch, encode_constellation_base, encode_slot_vector, encode_write_batch,
};
use calyx_aster::vault::ledger_stub;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::{Wal, WalOptions, replay_dir};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
    VaultStore,
};
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
    )
    .map_err(|error| error.to_string())?;
    for index in 1..=PRECOMMITTED_RECORDS {
        writer
            .put(synthetic_constellation(vault_id, index))
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())?;
    drop(writer);

    let target = synthetic_constellation(vault_id, TARGET_RECORD);
    let rows = constellation_rows(&target)?;
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
        store
            .load_current()
            .map_err(|error| error.to_string())?
            .durable_seq
    } else {
        0
    };
    let replay = replay_dir(vault.join("wal")).map_err(|error| error.to_string())?;
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
        let rows = decode_write_batch(&record.payload).map_err(|error| error.to_string())?;
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
    )
    .map_err(|error| error.to_string())?;
    let snapshot = opened.snapshot();
    let got = opened
        .get(id, snapshot)
        .map_err(|error| error.to_string())?;
    if got != expected {
        return Err(format!("open-check mismatch for index {index}").into());
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

fn ensure_empty_vault(vault: &Path) -> std::result::Result<(), String> {
    if !vault.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(vault).map_err(|error| error.to_string())?;
    if entries
        .next()
        .transpose()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err(format!(
            "crash-drill vault must be absent or empty: {}",
            vault.display()
        ));
    }
    Ok(())
}

fn append_torn_header(vault: &Path, seq: u64) -> std::result::Result<(), String> {
    let segment = vault.join("wal").join("00000000000000000000.wal");
    let mut file = OpenOptions::new()
        .append(true)
        .open(&segment)
        .map_err(|error| error.to_string())?;
    let start = file.metadata().map_err(|error| error.to_string())?.len();
    file.write_all(b"CXW1").map_err(|error| error.to_string())?;
    file.write_all(&seq.to_le_bytes())
        .map_err(|error| error.to_string())?;
    file.write_all(&64_u32.to_le_bytes())
        .map_err(|error| error.to_string())?;
    println!(
        "CRASH_TORN_WAL\tSEQ\t{}\tFILE\t{}\tSTART\t{}\tBYTES_WRITTEN\t16",
        seq,
        segment.display(),
        start
    );
    Ok(())
}

fn append_wal_batch(
    vault: &Path,
    expected_seq: u64,
    rows: &[WriteRow],
) -> std::result::Result<(), String> {
    let mut wal =
        Wal::open(vault.join("wal"), WalOptions::default()).map_err(|error| error.to_string())?;
    let payload = encode_write_batch(rows).map_err(|error| error.to_string())?;
    let ack = wal.append(&payload).map_err(|error| error.to_string())?;
    if ack.seq != expected_seq {
        return Err(format!(
            "unexpected WAL seq {}, expected {expected_seq}",
            ack.seq
        ));
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

fn write_batch_ssts(vault: &Path, seq: u64, rows: &[WriteRow], tag: &str) -> Result<usize, String> {
    for (index, row) in rows.iter().enumerate() {
        let dir = vault.join("cf").join(row.cf.name());
        fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let path = dir.join(format!("{seq:020}-{index:04}.sst"));
        let summary = write_sst(&path, [(row.key.as_slice(), row.value.as_slice())])
            .map_err(|error| error.to_string())?;
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

fn write_manifest(vault: &Path, seq: u64) -> Result<calyx_aster::manifest::ManifestWrite, String> {
    let (panel_ref, codebook_refs) = ensure_manifest_assets(vault)?;
    let manifest = VaultManifest::new(seq, seq, panel_ref, codebook_refs)
        .map_err(|error| error.to_string())?;
    ManifestStore::open(vault)
        .write_current(&manifest)
        .map_err(|error| error.to_string())
}

fn ensure_manifest_assets(vault: &Path) -> Result<(ImmutableRef, Vec<ImmutableRef>), String> {
    let panel_path = vault.join("panel/current.bin");
    let codebook_path = vault.join("codebooks/default.bin");
    let panel_bytes = b"calyx-stage1-panel";
    let codebook_bytes = b"calyx-stage1-codebook";
    write_asset(&panel_path, panel_bytes)?;
    write_asset(&codebook_path, codebook_bytes)?;
    Ok((
        ImmutableRef::from_bytes("panel/current.bin", panel_bytes)
            .map_err(|error| error.to_string())?,
        vec![
            ImmutableRef::from_bytes("codebooks/default.bin", codebook_bytes)
                .map_err(|error| error.to_string())?,
        ],
    ))
}

fn write_asset(path: &Path, bytes: &[u8]) -> std::result::Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut file = File::create(path).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    Ok(())
}

fn constellation_rows(cx: &Constellation) -> Result<Vec<WriteRow>, String> {
    let mut rows = vec![WriteRow {
        cf: ColumnFamily::Base,
        key: base_key(cx.cx_id),
        value: encode_constellation_base(cx).map_err(|error| error.to_string())?,
    }];
    for (slot, vector) in &cx.slots {
        rows.push(WriteRow {
            cf: ColumnFamily::slot(*slot),
            key: slot_key(cx.cx_id),
            value: encode_slot_vector(vector).map_err(|error| error.to_string())?,
        });
    }
    rows.push(WriteRow {
        cf: ColumnFamily::Ledger,
        key: ledger_key(cx.provenance.seq),
        value: ledger_stub::encode(cx.provenance.seq),
    });
    Ok(rows)
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

fn crash_vault_id() -> Result<VaultId, String> {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .map_err(|error| format!("crash vault id parse: {error}"))
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
