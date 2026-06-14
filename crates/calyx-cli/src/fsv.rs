use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::sst::arrow::{decode_column_chunk, encode_column_chunk};
use calyx_aster::sst::level::SstLevel;
use calyx_aster::sst::{SstReader, write_sst};
use calyx_aster::vault::AsterVault;
use calyx_aster::wal::{GroupCommitBatcher, Wal, WalOptions, replay_dir};
use calyx_core::{
    AbsentReason, Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector,
    SystemClock, VaultId, VaultStore,
};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

use crate::cf_read::{hex_bytes, list_sst_files};

pub fn arrow_demo(vault: &Path) -> crate::error::CliResult {
    let cf = ColumnFamily::slot(SlotId::new(0));
    let cf_dir = vault.join("cf").join(cf.name());
    fs::create_dir_all(&cf_dir).map_err(|error| error.to_string())?;
    let rows = [[1.0_f32, 2.0, 3.5, 4.25], [5.0, 6.0, 7.0, 8.0]];
    let refs: Vec<_> = rows.iter().map(|row| row.as_slice()).collect();
    let chunk = encode_column_chunk(&refs).map_err(|error| error.to_string())?;
    let decoded = decode_column_chunk(&chunk).map_err(|error| error.to_string())?;
    if decoded.n_rows() != 2 || decoded.dim() != 4 {
        return Err("arrow demo decoded unexpected shape".to_string().into());
    }
    let path = cf_dir.join("00000000000000000001.sst");
    let summary = write_sst(&path, [(b"arrow-key".as_slice(), chunk.as_slice())])
        .map_err(|error| error.to_string())?;
    let stored = SstReader::open(&path)
        .and_then(|reader| reader.get(b"arrow-key"))
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "arrow demo SST row missing".to_string())?;
    let stored = decode_column_chunk(&stored).map_err(|error| error.to_string())?;
    println!(
        "ARROW_DEMO\tCF\t{}\tSST\t{}\tKEY\t{}\tVALUE_MAGIC\t{}\tROWS\t{}\tDIM\t{}\tBYTES\t{}",
        cf.name(),
        summary.path.display(),
        hex_bytes(b"arrow-key"),
        hex_bytes(&stored.raw_bytes()[0..4]),
        stored.n_rows(),
        stored.dim(),
        summary.bytes
    );
    Ok(())
}

pub fn cf_demo(vault: &Path) -> crate::error::CliResult {
    let mut router = CfRouter::open(vault, 1024).map_err(|error| error.to_string())?;
    router
        .put(ColumnFamily::Base, b"k1", b"base-old")
        .map_err(|error| error.to_string())?;
    router
        .flush_cf(ColumnFamily::Base)
        .map_err(|error| error.to_string())?;
    router
        .put(ColumnFamily::Base, b"k1", b"base-new")
        .map_err(|error| error.to_string())?;
    router
        .put(ColumnFamily::Base, b"k2", b"base-two")
        .map_err(|error| error.to_string())?;
    router
        .put(ColumnFamily::slot(SlotId::new(0)), b"k1", b"slot-zero")
        .map_err(|error| error.to_string())?;
    router
        .flush_cf(ColumnFamily::Base)
        .map_err(|error| error.to_string())?;
    router
        .flush_cf(ColumnFamily::slot(SlotId::new(0)))
        .map_err(|error| error.to_string())?;
    drop(router);

    let reopened = CfRouter::open(vault, 1024).map_err(|error| error.to_string())?;
    assert_value(&reopened, ColumnFamily::Base, b"k1", b"base-new")?;
    assert_value(&reopened, ColumnFamily::Base, b"k2", b"base-two")?;
    assert_value(
        &reopened,
        ColumnFamily::slot(SlotId::new(0)),
        b"k1",
        b"slot-zero",
    )?;
    let base_rows = reopened
        .range(ColumnFamily::Base, b"", b"\xff")
        .map_err(|error| error.to_string())?;
    println!(
        "CF_DEMO\tVAULT\t{}\tBASE_FILES\t{}\tSLOT_FILES\t{}\tBASE_ROWS\t{}\tBASE_DIR\t{}\tSLOT_DIR\t{}",
        vault.display(),
        reopened.level_file_count(ColumnFamily::Base),
        reopened.level_file_count(ColumnFamily::slot(SlotId::new(0))),
        base_rows.len(),
        vault.join("cf/base").display(),
        vault.join("cf/slot_00").display()
    );
    Ok(())
}

pub fn mvcc_demo(vault: &Path) -> crate::error::CliResult {
    let vault_id = mvcc_vault_id()?;
    let router = CfRouter::open(vault, 4096).map_err(|error| error.to_string())?;
    let writer = AsterVault::with_clock_and_router(
        vault_id,
        b"calyx-mvcc-router-demo-salt".to_vec(),
        SystemClock,
        router,
    );
    let constellation = mvcc_constellation(vault_id);
    let id = constellation.cx_id;
    writer
        .put(constellation)
        .map_err(|error| error.to_string())?;
    let summaries = writer.flush_all_cfs().map_err(|error| error.to_string())?;
    let stale = writer.pin_stale_snapshot(3);
    println!(
        "MVCC_DEMO\tID\t{}\tSNAPSHOT\t{}\tSTALE_MAX_LAG\t3\tFLUSHED\t{}\tBASE_CF\t{}\tSLOT_CF\t{}",
        id,
        stale.seq(),
        summaries.len(),
        vault.join("cf/base").display(),
        vault.join("cf/slot_00").display()
    );
    Ok(())
}

pub fn wal_drill(vault: &Path, records: usize) -> crate::error::CliResult {
    let wal_dir = vault.join("wal");
    fs::create_dir_all(&wal_dir).map_err(|error| error.to_string())?;
    let options = WalOptions::default();
    let wal = Wal::open(&wal_dir, options).map_err(|error| error.to_string())?;
    let batcher = GroupCommitBatcher::new(wal, options.group_commit_window, Arc::new(SystemClock))
        .map_err(|error| error.to_string())?;
    let mut last_seq = 0;
    let mut last_end = 0;
    for index in 0..records {
        let ack = batcher
            .submit(format!("acked-{index:04}").into_bytes())
            .map_err(|error| error.to_string())?;
        last_seq = ack.seq;
        last_end = ack.end_offset;
    }
    batcher.flush_sync().map_err(|error| error.to_string())?;
    drop(batcher);

    let segment = wal_dir.join("00000000000000000000.wal");
    let torn_seq = last_seq + 1;
    let mut file = OpenOptions::new()
        .append(true)
        .open(&segment)
        .map_err(|error| error.to_string())?;
    file.write_all(b"CXW1").map_err(|error| error.to_string())?;
    file.write_all(&torn_seq.to_le_bytes())
        .map_err(|error| error.to_string())?;
    file.write_all(&32_u32.to_le_bytes())
        .map_err(|error| error.to_string())?;
    println!(
        "WAL_DRILL\tLAST_ACKED_SEQ\t{}\tTORN_SEQ\t{}\tLAST_ACKED_END\t{}\tWAL_DIR\t{}\tSEGMENT\t{}",
        last_seq,
        torn_seq,
        last_end,
        wal_dir.display(),
        segment.display()
    );
    Ok(())
}

pub fn wal_replay(wal_dir: &Path) -> crate::error::CliResult {
    let replay = replay_dir(wal_dir).map_err(|error| error.to_string())?;
    for record in replay.records {
        println!(
            "WAL_REPLAY\tSEQ\t{}\tFILE\t{}\tSTART\t{}\tEND\t{}\tPAYLOAD\t{}",
            record.seq,
            record.segment_path.display(),
            record.start_offset,
            record.end_offset,
            hex_bytes(&record.payload)
        );
    }
    if let Some(torn) = replay.torn_tail {
        println!(
            "WAL_TORN\tCODE\t{}\tFILE\t{}\tOFFSET\t{}\tMESSAGE\t{}",
            torn.code,
            torn.segment_path.display(),
            torn.offset,
            torn.message
        );
    }
    Ok(())
}

pub fn corrupt_shard(vault: &Path, cf_name: &str, byte_offset: u64) -> crate::error::CliResult {
    let cf = parse_cf(cf_name)?;
    let files = list_sst_files(&vault.join("cf").join(cf.name()))?;
    let path = files
        .first()
        .ok_or_else(|| format!("no SST files for {}", cf.name()))?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    let len = file.metadata().map_err(|error| error.to_string())?.len();
    if byte_offset >= len {
        return Err(format!("byte offset {byte_offset} outside SST length {len}").into());
    }
    file.seek(SeekFrom::Start(byte_offset))
        .map_err(|error| error.to_string())?;
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte)
        .map_err(|error| error.to_string())?;
    byte[0] ^= 0xff;
    file.seek(SeekFrom::Start(byte_offset))
        .map_err(|error| error.to_string())?;
    file.write_all(&byte).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    println!(
        "CORRUPT_SHARD\tCF\t{}\tFILE\t{}\tOFFSET\t{}\tXOR\tff",
        cf.name(),
        path.display(),
        byte_offset
    );
    Ok(())
}

pub fn readback_level(cf_name: &str, level_dir: &Path) -> crate::error::CliResult {
    let cf = parse_cf(cf_name)?;
    let files = list_sst_files(level_dir)?;
    let level = SstLevel::from_oldest_first(files.clone());
    for row in level
        .range(b"", b"\xff")
        .map_err(|error| error.to_string())?
    {
        println!(
            "LEVEL\tCF\t{}\tFILES\t{}\tKEY\t{}\tVALUE\t{}",
            cf.name(),
            files.len(),
            hex_bytes(&row.key),
            hex_bytes(&row.value)
        );
    }
    Ok(())
}

fn assert_value(
    router: &CfRouter,
    cf: ColumnFamily,
    key: &[u8],
    expected: &[u8],
) -> crate::error::CliResult {
    let got = router
        .get(cf, key)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("missing {} key {}", cf.name(), hex_bytes(key)))?;
    if got != expected {
        return Err(format!("{} key {} mismatch", cf.name(), hex_bytes(key)).into());
    }
    Ok(())
}

fn parse_cf(value: &str) -> Result<ColumnFamily, String> {
    match value {
        "base" => Ok(ColumnFamily::Base),
        "graph" => Ok(ColumnFamily::Graph),
        "slot_00" => Ok(ColumnFamily::slot(SlotId::new(0))),
        _ => Err(format!("unsupported FSV column family: {value}")),
    }
}

fn mvcc_vault_id() -> Result<VaultId, String> {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .map_err(|error| format!("demo vault id parse: {error}"))
}

fn mvcc_constellation(vault_id: VaultId) -> Constellation {
    let cx_id = calyx_core::CxId::from_bytes([0x88; 16]);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 3,
            data: vec![0.125, 0.25, 0.5],
        },
    );
    slots.insert(
        SlotId::new(1),
        SlotVector::Absent {
            reason: AbsentReason::Deferred,
        },
    );
    Constellation {
        cx_id,
        vault_id,
        panel_version: 22,
        created_at: 1780822800,
        input_ref: InputRef {
            hash: [0x88; 32],
            pointer: Some("synthetic://calyx-mvcc-router-demo".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [0x44; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_bytes_matches_lowercase_plain_hex() {
        assert_eq!(hex_bytes(b"k1"), "6b31");
    }

    #[test]
    fn fsv_cf_parser_names_supported_demo_cfs() {
        assert_eq!(parse_cf("base").unwrap(), ColumnFamily::Base);
        assert_eq!(
            parse_cf("slot_00").unwrap(),
            ColumnFamily::slot(SlotId::new(0))
        );
        assert!(parse_cf("slot_01").is_err());
    }
}
