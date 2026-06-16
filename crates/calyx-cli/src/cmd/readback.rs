use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::manifest::ManifestStore;
use calyx_aster::sst::SstReader;
use calyx_aster::storage_names::wal_segment_index;
use calyx_aster::vault::encode::decode_write_batch;
use calyx_core::CalyxError;

use crate::cf_read::{hex_bytes, list_sst_files};
use crate::error::{CliError, CliResult};
use crate::output::print_hex_dump;
use crate::{ops, vault_tree};

const WAL_MAGIC: u32 = u32::from_le_bytes(*b"CXW1");
const WAL_HEADER_LEN: usize = 20;
const WAL_MAX_RECORD_BYTES: u32 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReadbackCommand {
    Hex(PathBuf),
    VaultTree(PathBuf),
    CfRow {
        vault: PathBuf,
        cf: String,
        key_hex: String,
    },
    WalSegment(PathBuf),
    Ledger {
        vault: PathBuf,
        seq: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WalRecord {
    seq: u64,
    len: u32,
    crc: u32,
    payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum WalEvent {
    Record(WalRecord),
    TornTail { seq: Option<u64> },
}

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    if args.first().map(String::as_str) != Some("readback") {
        return None;
    }
    if !owns_form(args) {
        return None;
    }
    Some(parse(args).and_then(run))
}

fn owns_form(args: &[String]) -> bool {
    matches!(
        args.get(1).map(String::as_str),
        Some("--hex" | "--vault-tree" | "--cf-row" | "--ledger")
    ) || matches!(args.get(1).map(String::as_str), Some("--wal")) && args.len() == 3
}

fn parse(args: &[String]) -> CliResult<ReadbackCommand> {
    match args {
        [_, flag, path] if flag == "--hex" => Ok(ReadbackCommand::Hex(path.into())),
        [_, flag, path] if flag == "--vault-tree" => Ok(ReadbackCommand::VaultTree(path.into())),
        [_, flag, path] if flag == "--wal" => Ok(ReadbackCommand::WalSegment(path.into())),
        [_, flag, vault, cf_flag, cf, key_flag, key]
            if flag == "--cf-row" && cf_flag == "--cf" && key_flag == "--key" =>
        {
            Ok(ReadbackCommand::CfRow {
                vault: vault.into(),
                cf: cf.clone(),
                key_hex: key.clone(),
            })
        }
        [_, flag, vault, seq_flag, seq] if flag == "--ledger" && seq_flag == "--seq" => {
            Ok(ReadbackCommand::Ledger {
                vault: vault.into(),
                seq: parse_seq(seq)?,
            })
        }
        _ => Err(CliError::usage(
            "usage: calyx readback (--hex <file> | --vault-tree <dir> | --cf-row <vault> --cf <cf-name> --key <hex-key> | --wal <segment-path> | --ledger <vault> --seq <n>)",
        )),
    }
}

fn run(command: ReadbackCommand) -> CliResult {
    match command {
        ReadbackCommand::Hex(path) => readback_hex(&path),
        ReadbackCommand::VaultTree(path) => vault_tree::readback_vault_tree(&path),
        ReadbackCommand::CfRow { vault, cf, key_hex } => readback_cf_row(&vault, &cf, &key_hex),
        ReadbackCommand::WalSegment(path) => readback_wal_segment(&path),
        ReadbackCommand::Ledger { vault, seq } => readback_ledger(&vault, seq),
    }
}

fn readback_hex(path: &Path) -> CliResult {
    let bytes = fs::read(path)?;
    print_hex_dump(0, &bytes);
    Ok(())
}

fn readback_cf_row(vault: &Path, cf_name: &str, key_hex: &str) -> CliResult {
    ensure_manifested_vault(vault)?;
    let cf = ops::parse_cf(cf_name).map_err(CliError::usage)?;
    let key = parse_hex_bytes(key_hex, "--key")?;
    let value = latest_cf_row(vault, cf, &key)?.ok_or_else(|| {
        CalyxError::aster_corrupt_shard(format!(
            "CF {} row key {} not found",
            cf.name(),
            hex_bytes(&key)
        ))
    })?;
    print_hex_dump(0, &value);
    Ok(())
}

fn readback_ledger(vault: &Path, seq: u64) -> CliResult {
    ensure_manifested_vault(vault)?;
    let key = seq.to_be_bytes();
    let bytes = latest_cf_row(vault, ColumnFamily::Ledger, &key)?.ok_or_else(|| {
        CalyxError::vault_access_denied(format!("ledger seq {seq} does not exist"))
    })?;
    let entry = calyx_ledger::decode(&bytes)?;
    println!(
        "LEDGER seq={} prev_hash={} entry_hash={} kind={}",
        entry.seq,
        hex_bytes(&entry.prev_hash),
        hex_bytes(&entry.entry_hash),
        entry.kind
    );
    print_hex_dump(0, &bytes);
    Ok(())
}

fn readback_wal_segment(path: &Path) -> CliResult {
    for event in wal_events(path)? {
        match event {
            WalEvent::Record(record) => {
                println!(
                    "WAL seq={} group=0 len={} crc={:08x}",
                    record.seq, record.len, record.crc
                );
                print_hex_dump(0, &record.payload);
            }
            WalEvent::TornTail { seq } => match seq {
                Some(seq) => println!("TORN_TAIL seq={seq}"),
                None => println!("TORN_TAIL seq=unknown"),
            },
        }
    }
    Ok(())
}

fn latest_cf_row(vault: &Path, cf: ColumnFamily, key: &[u8]) -> CliResult<Option<Vec<u8>>> {
    let mut value = None;
    for file in list_sst_files(&vault.join("cf").join(cf.name()))? {
        let reader = SstReader::open(&file)?;
        if let Some(bytes) = reader.get(key)? {
            value = Some(bytes);
        }
    }
    for path in wal_segment_paths(&vault.join("wal"))? {
        for event in wal_events(&path)? {
            let WalEvent::Record(record) = event else {
                continue;
            };
            for row in decode_write_batch(&record.payload)? {
                if row.cf == cf && row.key == key {
                    value = Some(row.value);
                }
            }
        }
    }
    Ok(value)
}

fn wal_segment_paths(dir: &Path) -> CliResult<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if wal_segment_index(&path)?.is_some() {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn wal_events(path: &Path) -> CliResult<Vec<WalEvent>> {
    let bytes = fs::read(path)?;
    let mut events = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        if remaining < WAL_HEADER_LEN {
            events.push(WalEvent::TornTail { seq: None });
            break;
        }
        let header = &bytes[offset..offset + WAL_HEADER_LEN];
        let magic = u32::from_le_bytes(header[0..4].try_into().expect("magic width"));
        if magic != WAL_MAGIC {
            return Err(CalyxError::aster_torn_wal(format!(
                "{} bad WAL magic 0x{magic:08x} at byte {offset}",
                path.display()
            ))
            .into());
        }
        let seq = u64::from_le_bytes(header[4..12].try_into().expect("seq width"));
        let len = u32::from_le_bytes(header[12..16].try_into().expect("len width"));
        let crc = u32::from_le_bytes(header[16..20].try_into().expect("crc width"));
        if len > WAL_MAX_RECORD_BYTES {
            return Err(CalyxError::aster_torn_wal(format!(
                "{} WAL record seq {seq} length {len} exceeds max {WAL_MAX_RECORD_BYTES}",
                path.display()
            ))
            .into());
        }
        let payload_start = offset + WAL_HEADER_LEN;
        let payload_end = payload_start + len as usize;
        if payload_end > bytes.len() {
            events.push(WalEvent::TornTail { seq: Some(seq) });
            break;
        }
        let payload = bytes[payload_start..payload_end].to_vec();
        let actual = wal_payload_crc(seq, len, &payload);
        if actual != crc {
            return Err(CalyxError::aster_torn_wal(format!(
                "{} WAL crc mismatch for seq {seq}: expected {crc:08x}, got {actual:08x}",
                path.display()
            ))
            .into());
        }
        events.push(WalEvent::Record(WalRecord {
            seq,
            len,
            crc,
            payload,
        }));
        offset = payload_end;
    }
    Ok(events)
}

fn wal_payload_crc(seq: u64, len: u32, payload: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&seq.to_le_bytes());
    hasher.update(&len.to_le_bytes());
    hasher.update(payload);
    hasher.finalize()
}

fn ensure_manifested_vault(vault: &Path) -> CliResult {
    if !vault.is_dir() || !vault.join("CURRENT").is_file() || !vault.join("MANIFEST").is_file() {
        return Err(CliError::usage(format!(
            "vault path {} is not a valid Calyx vault (missing manifest)",
            vault.display()
        )));
    }
    ManifestStore::open(vault).load_current()?;
    Ok(())
}

fn parse_seq(value: &str) -> CliResult<u64> {
    value
        .parse::<u64>()
        .map_err(|error| CliError::usage(format!("invalid --seq {value}: {error}")))
}

fn parse_hex_bytes(value: &str, label: &str) -> CliResult<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(CliError::usage(format!(
            "{label} must contain an even number of hex digits"
        )));
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks(2) {
        let high = hex_value(pair[0])
            .ok_or_else(|| CliError::usage(format!("{label} contains non-hex digit")))?;
        let low = hex_value(pair[1])
            .ok_or_else(|| CliError::usage(format!("{label} contains non-hex digit")))?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
