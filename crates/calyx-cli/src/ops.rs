use calyx_aster::cf::ColumnFamily;
use calyx_aster::compaction::{
    CompactionResult, CompactionThrottle, SstShard, StorageTier, TieringPolicy, compact_shards,
};
use calyx_aster::sst::{SstReader, write_sst};
use calyx_aster::wal::{Wal, WalOptions, replay_dir};
use calyx_core::SlotId;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

pub fn compact(vault: &Path, cf_name: &str) -> Result<(), String> {
    let cf = parse_cf(cf_name)?;
    let cf_dir = vault.join("cf").join(cf.name());
    let files = list_sst_files(&cf_dir)?;
    let shards = shards_for(cf, &files)?;
    let output = cf_dir.join(format!("compact-{}.sst", unix_millis()));
    let result = compact_shards(cf, &shards, &output, CompactionThrottle::unlimited())
        .map_err(|error| error.to_string())?;
    match result {
        CompactionResult::Skipped { debt } => {
            println!(
                "COMPACT_SKIPPED\tCF\t{}\tPENDING_BYTES\t{}\tSCORE_MILLI\t{}",
                cf.name(),
                debt.pending_bytes,
                debt.score_milli
            );
        }
        CompactionResult::Compacted(report) => {
            for file in files {
                if file != report.output_path {
                    fs::remove_file(&file)
                        .map_err(|error| format!("remove compacted input: {error}"))?;
                }
            }
            print_report("COMPACTED", &report);
        }
    }
    Ok(())
}

pub fn compact_watch(vault: &Path, duration: Duration) -> Result<(), String> {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        compact(vault, "base")?;
        thread::sleep(Duration::from_millis(500));
    }
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
        .write_tiered_sst(cf, panel_version, "tiered.sst", [(b"k".as_slice(), b"tier".as_slice())])
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

    let mut wal = Wal::open(vault.join("wal"), WalOptions::default())
        .map_err(|error| error.to_string())?;
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

fn shards_for(cf: ColumnFamily, files: &[PathBuf]) -> Result<Vec<SstShard>, String> {
    files
        .iter()
        .map(|file| SstShard::new(cf, file, 0).map_err(|error| error.to_string()))
        .collect()
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
        "ledger" => Ok(ColumnFamily::Ledger),
        "online" => Ok(ColumnFamily::Online),
        "scalars" => Ok(ColumnFamily::Scalars),
        "xterm" => Ok(ColumnFamily::XTerm),
        _ if value.starts_with("slot_") => parse_slot_cf(value),
        _ => Err(format!("unknown column family: {value}")),
    }
}

fn parse_slot_cf(value: &str) -> Result<ColumnFamily, String> {
    let raw = value.ends_with(".raw");
    let slot_text = value
        .trim_start_matches("slot_")
        .trim_end_matches(".raw");
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

fn print_report(label: &str, report: &calyx_aster::compaction::CompactionReport) {
    println!(
        "{}\tCF\t{}\tINPUT_FILES\t{}\tINPUT_BYTES\t{}\tOUTPUT_BYTES\t{}\tLOGICAL_BYTES\t{}\tWRITE_AMP_MILLI\t{}\tOUTPUT\t{}\tSTAGING_PARENT\t{}",
        label,
        report.cf.name(),
        report.input_files,
        report.input_bytes,
        report.output_bytes,
        report.logical_bytes,
        report.write_amp_milli,
        report.output_path.display(),
        report.staging_parent.display()
    );
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn fd_count() -> usize {
    fs::read_dir("/proc/self/fd").map(|entries| entries.count()).unwrap_or(0)
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
