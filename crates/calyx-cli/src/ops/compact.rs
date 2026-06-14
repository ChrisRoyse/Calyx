use super::{parse_cf, unix_millis};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::compaction::{
    CompactionReport, CompactionResult, CompactionThrottle, SstShard, compact_shards,
};
use calyx_aster::manifest::ManifestStore;
use calyx_aster::storage_names::{SstName, classify_sst};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cf_read::list_sst_files;

const DURABLE_COMPACT_FIRST_INDEX: u16 = 9_000;
const DURABLE_COMPACT_LAST_INDEX: u16 = 9_999;

pub fn compact(vault: &Path, cf_name: &str) -> crate::error::CliResult {
    let cf = parse_cf(cf_name)?;
    let cf_dir = vault.join("cf").join(cf.name());
    let files = list_sst_files(&cf_dir)?;
    let output = compaction_output_path(vault, &cf_dir, &files)?;
    let shards = shards_for(cf, &files)?;
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
            remove_compacted_inputs(&files, &report)?;
            print_report("COMPACTED", &report);
        }
    }
    Ok(())
}

fn compaction_output_path(
    vault: &Path,
    cf_dir: &Path,
    files: &[PathBuf],
) -> Result<PathBuf, String> {
    if !vault.join("CURRENT").exists() {
        // Canonical compacted-class name so fail-closed scans accept it.
        return Ok(cf_dir.join(format!("compacted-{:020}.sst", unix_millis())));
    }
    let durable_seq = ManifestStore::open(vault)
        .load_current()
        .map_err(|error| format!("load durable manifest for CLI compact: {error}"))?
        .durable_seq;
    validate_durable_inputs(files, durable_seq)?;
    if durable_seq == 0 && files.len() >= 2 {
        return Err("refusing durable CLI compact before CURRENT durable_seq advances".to_string());
    }
    durable_compaction_output_path(cf_dir, durable_seq)
}

fn validate_durable_inputs(files: &[PathBuf], durable_seq: u64) -> Result<(), String> {
    let hidden = files
        .iter()
        .filter(|file| !durable_input_is_manifest_bounded(file, durable_seq))
        .map(|file| file.display().to_string())
        .collect::<Vec<_>>();
    if hidden.is_empty() {
        return Ok(());
    }
    Err(format!(
        "refusing durable CLI compact; {} SST file(s) are not bounded by CURRENT durable_seq {}: {}",
        hidden.len(),
        durable_seq,
        hidden.join(", ")
    ))
}

fn durable_compaction_output_path(cf_dir: &Path, durable_seq: u64) -> Result<PathBuf, String> {
    for index in (DURABLE_COMPACT_FIRST_INDEX..=DURABLE_COMPACT_LAST_INDEX).rev() {
        let path = cf_dir.join(format!("{durable_seq:020}-{index:04}.sst"));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(format!(
        "no durable CLI compaction output slot remains for seq {durable_seq}"
    ))
}

fn durable_input_is_manifest_bounded(path: &Path, durable_seq: u64) -> bool {
    match classify_sst(path) {
        Ok(Some(
            SstName::Router { seq }
            | SstName::DurableBatch { seq, .. }
            | SstName::Compacted { seq },
        )) => seq <= durable_seq,
        _ => false,
    }
}

fn shards_for(cf: ColumnFamily, files: &[PathBuf]) -> Result<Vec<SstShard>, String> {
    files
        .iter()
        .map(|file| SstShard::new(cf, file, 0).map_err(|error| error.to_string()))
        .collect()
}

fn remove_compacted_inputs(
    files: &[PathBuf],
    report: &CompactionReport,
) -> std::result::Result<(), String> {
    for file in files {
        if file != &report.output_path {
            fs::remove_file(file).map_err(|error| format!("remove compacted input: {error}"))?;
        }
    }
    Ok(())
}

fn print_report(label: &str, report: &CompactionReport) {
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
