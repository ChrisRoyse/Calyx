use calyx_aster::cf::ColumnFamily;
use calyx_aster::compaction::{
    CompactionScheduler, CompactionSchedulerOptions, StorageTier, TieringPolicy,
    catalog_from_vault_dir,
};
use calyx_aster::sst::{SstReader, write_sst};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::{
    DEFAULT_GROUP_COMMIT_WINDOW, GroupCommitBatcher, Wal, WalOptions, replay_dir,
};
use calyx_core::{
    AbsentReason, Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector,
    SystemClock, VaultId, VaultStore,
};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod compact;
pub use compact::compact;

const SOAK_VALUE_BYTES: usize = 256;

pub fn readback_cf(vault: &Path, cf_name: &str) -> Result<(), String> {
    let cf = parse_cf(cf_name)?;
    let files = list_sst_files(&vault.join("cf").join(cf.name()))?;
    for file in files {
        let reader = SstReader::open(&file).map_err(|error| error.to_string())?;
        for row in reader.iter().map_err(|error| error.to_string())? {
            println!(
                "CF\t{}\tFILE\t{}\tKEY\t{}\tVALUE\t{}",
                cf.name(),
                file.display(),
                hex_bytes(&row.key),
                hex_bytes(&row.value)
            );
        }
    }
    Ok(())
}

pub fn readback_wal(vault: &Path) -> Result<(), String> {
    let replay = replay_dir(vault.join("wal")).map_err(|error| error.to_string())?;
    for record in replay.records {
        println!(
            "WAL\tSEQ\t{}\tFILE\t{}\tSTART\t{}\tEND\t{}\tPAYLOAD\t{}",
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

pub fn compact_watch(vault: &Path, duration: Duration) -> Result<(), String> {
    let catalog = Arc::new(catalog_from_vault_dir(vault).map_err(|error| error.to_string())?);
    let options = CompactionSchedulerOptions {
        interval_ms: 1,
        debt_trigger_score_milli: 0,
        output_root: vault.join("cf"),
        ..CompactionSchedulerOptions::default()
    };
    let scheduler = CompactionScheduler::start(catalog.clone(), options);
    thread::sleep(duration);
    scheduler
        .stop()
        .map_err(|_| "compaction scheduler panicked".to_string())?;
    println!(
        "COMPACT_WATCH_DONE\tVAULT\t{}\tBASE_SHARDS\t{}",
        vault.display(),
        catalog.shard_count_for_cf(ColumnFamily::Base)
    );
    Ok(())
}

pub fn tier(vault: &Path, cf_name: &str, output: &str) -> Result<(), String> {
    let cf = parse_cf(cf_name)?;
    let (hot, archive) = tier_roots(vault);
    let policy = TieringPolicy::new(hot, archive, [SlotId::new(0), SlotId::new(1)], 1);
    let panel_version = match output {
        "hot" => 1,
        "cold" => 0,
        _ => return Err("tier output must be hot or cold".to_string()),
    };
    let written = policy
        .write_tiered_sst(
            cf,
            panel_version,
            "tiered.sst",
            [(b"k".as_slice(), b"tier".as_slice())],
        )
        .map_err(|error| error.to_string())?;
    println!(
        "TIER_WRITE\tCF\t{}\tTIER\t{:?}\tPATH\t{}\tBYTES\t{}\tSTAGING_PARENT\t{}",
        cf.name(),
        written.placement.tier,
        written.path.display(),
        written.bytes,
        written.staging_parent.display()
    );
    if output == "cold" && written.placement.tier != StorageTier::Cold {
        return Err(format!("{} did not resolve to cold tier", cf.name()));
    }
    if output == "hot" && written.placement.tier != StorageTier::Hot {
        return Err(format!("{} did not resolve to hot tier", cf.name()));
    }
    Ok(())
}

pub fn soak(vault: &Path, ops: usize, threads: usize) -> Result<(), String> {
    let fd_before = fd_count();
    fs::create_dir_all(vault.join("cf/base")).map_err(|error| error.to_string())?;
    fs::create_dir_all(vault.join("wal")).map_err(|error| error.to_string())?;
    if ops == 0 {
        println!("SOAK\tOPS\t0\tWRITE_AMP_MILLI\t1000");
        println!("FD_COUNT\tBEFORE\t{fd_before}\tAFTER\t{}", fd_count());
        return Ok(());
    }

    let workers = threads.max(1);
    let mut handles = Vec::with_capacity(workers);
    for worker in 0..workers {
        let path = vault.join("cf/base").join(format!("soak-{worker:02}.sst"));
        let entries = soak_entries(worker, workers, ops);
        handles.push(thread::spawn(move || write_entries(path, entries)));
    }
    for handle in handles {
        handle
            .join()
            .map_err(|_| "soak worker panicked".to_string())??;
    }

    let mut wal =
        Wal::open(vault.join("wal"), WalOptions::default()).map_err(|error| error.to_string())?;
    for op in 0..ops {
        wal.append(&soak_value(op, 0))
            .map_err(|error| error.to_string())?;
    }
    drop(wal);

    compact(vault, "base")?;
    tier(vault, "slot_00.raw", "cold")?;
    println!(
        "SOAK_DONE\tOPS\t{}\tTHREADS\t{}\tFD_BEFORE\t{}\tFD_AFTER\t{}",
        ops,
        workers,
        fd_before,
        fd_count()
    );
    Ok(())
}

pub fn vault_demo(vault: &Path) -> Result<(), String> {
    let vault_id = demo_vault_id()?;
    let writer = AsterVault::new_durable(
        vault,
        vault_id,
        b"calyx-cli-demo-salt".to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let constellation = demo_constellation(&writer, vault_id);
    let id = constellation.cx_id;

    writer
        .put(constellation.clone())
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())?;
    let reopened = AsterVault::open(
        vault,
        vault_id,
        b"calyx-cli-demo-salt".to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let got = reopened
        .get(id, reopened.snapshot())
        .map_err(|error| error.to_string())?;
    let mut expected = constellation;
    expected.provenance = got.provenance.clone();
    if got != expected {
        return Err("cold-open constellation mismatch".to_string());
    }

    println!(
        "VAULT_DEMO\tID\t{}\tSNAPSHOT\t{}\tWAL\t{}\tBASE_CF\t{}\tCURRENT\t{}",
        id,
        reopened.snapshot(),
        vault.join("wal/00000000000000000000.wal").display(),
        vault.join("cf/base").display(),
        vault.join("CURRENT").display()
    );
    Ok(())
}

pub fn wal_batch_demo(vault: &Path, requests: usize) -> Result<(), String> {
    fs::create_dir_all(vault.join("wal")).map_err(|error| error.to_string())?;
    let wal = Wal::open(vault.join("wal"), WalOptions::default()).map_err(|e| e.to_string())?;
    let batcher = Arc::new(
        GroupCommitBatcher::new(wal, DEFAULT_GROUP_COMMIT_WINDOW, Arc::new(SystemClock))
            .map_err(|error| error.to_string())?,
    );
    let mut handles = Vec::with_capacity(requests);
    for index in 0..requests {
        let batcher = batcher.clone();
        handles.push(thread::spawn(move || {
            batcher.submit(format!("batch-{index:04}").into_bytes())
        }));
    }
    let mut acks = Vec::with_capacity(requests);
    for handle in handles {
        acks.push(
            handle
                .join()
                .map_err(|_| "batch submitter panicked".to_string())?
                .map_err(|error| error.to_string())?,
        );
    }
    batcher.flush_sync().map_err(|error| error.to_string())?;
    acks.sort_by_key(|ack| ack.seq);
    for ack in &acks {
        println!(
            "WAL_BATCH_ACK\tSEQ\t{}\tFILE\t{}\tSTART\t{}\tEND\t{}",
            ack.seq,
            ack.segment_path.display(),
            ack.start_offset,
            ack.end_offset
        );
    }
    println!("WAL_BATCH_DONE\tREQUESTS\t{}", acks.len());
    Ok(())
}

pub fn parse_duration(value: &str) -> Result<Duration, String> {
    if let Some(ms) = value.strip_suffix("ms") {
        return ms
            .parse::<u64>()
            .map(Duration::from_millis)
            .map_err(|error| error.to_string());
    }
    if let Some(seconds) = value.strip_suffix('s') {
        return seconds
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|error| error.to_string());
    }
    value
        .parse::<u64>()
        .map(Duration::from_secs)
        .map_err(|error| error.to_string())
}

fn write_entries(path: PathBuf, mut entries: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), String> {
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let refs: Vec<_> = entries
        .iter()
        .map(|(key, value)| (key.as_slice(), value.as_slice()))
        .collect();
    write_sst(path, refs)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn soak_entries(worker: usize, workers: usize, ops: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    (0..ops)
        .filter(|op| op % workers == worker)
        .map(|op| ((op as u64).to_be_bytes().to_vec(), soak_value(op, worker)))
        .collect()
}

fn soak_value(op: usize, worker: usize) -> Vec<u8> {
    let mut value = vec![worker as u8; SOAK_VALUE_BYTES];
    value[0..8].copy_from_slice(&(op as u64).to_be_bytes());
    value
}

fn demo_vault_id() -> Result<VaultId, String> {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .map_err(|error| format!("demo vault id parse: {error}"))
}

fn demo_constellation(vault: &AsterVault, vault_id: VaultId) -> Constellation {
    let input = b"calyx durable cli demo";
    let cx_id = vault.cx_id_for_input(input, 11);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input);
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
        panel_version: 11,
        created_at: 1780822800,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some("synthetic://calyx-durable-cli-demo".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [7; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn list_sst_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("sst") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn parse_cf(value: &str) -> Result<ColumnFamily, String> {
    match value {
        "base" => Ok(ColumnFamily::Base),
        "anchors" => Ok(ColumnFamily::Anchors),
        "assay" => Ok(ColumnFamily::Assay),
        "ledger" => Ok(ColumnFamily::Ledger),
        "recurrence" => Ok(ColumnFamily::Recurrence),
        "graph" => Ok(ColumnFamily::Graph),
        "online" => Ok(ColumnFamily::Online),
        "scalars" => Ok(ColumnFamily::Scalars),
        "xterm" => Ok(ColumnFamily::XTerm),
        "temporal_xterm" => Ok(ColumnFamily::TemporalXTerm),
        "anneal_rollback" => Ok(ColumnFamily::AnnealRollback),
        "anneal_health" => Ok(ColumnFamily::AnnealHealth),
        "anneal_checksums" => Ok(ColumnFamily::AnnealChecksums),
        "anneal_mistakes" => Ok(ColumnFamily::AnnealMistakes),
        "anneal_replay" => Ok(ColumnFamily::AnnealReplay),
        "anneal_heads" => Ok(ColumnFamily::AnnealHeads),
        _ if value.starts_with("slot_") => parse_slot_cf(value),
        _ => Err(format!("unknown column family: {value}")),
    }
}

fn parse_slot_cf(value: &str) -> Result<ColumnFamily, String> {
    let raw = value.ends_with(".raw");
    let slot_text = value.trim_start_matches("slot_").trim_end_matches(".raw");
    let slot = slot_text
        .parse::<u16>()
        .map_err(|error| format!("invalid slot id {slot_text}: {error}"))?;
    if raw {
        Ok(ColumnFamily::slot_raw(SlotId::new(slot)))
    } else {
        Ok(ColumnFamily::slot(SlotId::new(slot)))
    }
}

fn tier_roots(vault: &Path) -> (PathBuf, PathBuf) {
    let home = env::var_os("CALYX_HOME")
        .map(PathBuf::from)
        .or_else(|| vault.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    (home.join("hot"), home.join("archive"))
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn fd_count() -> usize {
    fs::read_dir("/proc/self/fd")
        .map(|entries| entries.count())
        .unwrap_or(0)
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
