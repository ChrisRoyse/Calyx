use super::{parse_cf, unix_millis};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::compaction::{
    CompactionReport, CompactionResult, CompactionThrottle, SstShard, compact_shards,
    durable_compaction_slot_path,
};
use calyx_aster::manifest::ManifestStore;
use calyx_aster::storage_names::{SstName, classify_sst};
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{CliError, CliResult};

pub fn compact(vault: &Path, cf_name: &str) -> crate::error::CliResult {
    let cf = parse_cf(cf_name).map_err(CliError::usage)?;
    let cf_dir = vault.join("cf").join(cf.name());
    let files = list_sst_files_for_adoption(&cf_dir)?;
    let output = compaction_output_path(vault, &cf_dir, &files)?;
    let shards = shards_for(cf, &files)?;
    let result = compact_shards(cf, &shards, &output, CompactionThrottle::unlimited())?;
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

fn compaction_output_path(vault: &Path, cf_dir: &Path, files: &[PathBuf]) -> CliResult<PathBuf> {
    if !vault.join("CURRENT").exists() {
        // Canonical compacted-class name so fail-closed scans accept it.
        return Ok(cf_dir.join(format!("compacted-{:020}.sst", unix_millis())));
    }
    let durable_seq = ManifestStore::open(vault).load_current()?.durable_seq;
    validate_durable_inputs(files, durable_seq)?;
    if durable_seq == 0 && files.len() >= 2 {
        return Err(CliError::runtime(
            "refusing durable CLI compact before CURRENT durable_seq advances",
        ));
    }
    Ok(durable_compaction_slot_path(cf_dir, durable_seq)?)
}

fn validate_durable_inputs(files: &[PathBuf], durable_seq: u64) -> CliResult {
    let hidden = files
        .iter()
        .filter(|file| !durable_input_is_manifest_bounded(file, durable_seq))
        .map(|file| file.display().to_string())
        .collect::<Vec<_>>();
    if hidden.is_empty() {
        return Ok(());
    }
    Err(CliError::runtime(format!(
        "refusing durable CLI compact; {} SST file(s) are not bounded by CURRENT durable_seq {}: {}",
        hidden.len(),
        durable_seq,
        hidden.join(", ")
    )))
}

/// Lists a CF directory's canonical SST files in epoch order WITHOUT the
/// `CALYX_ASTER_SST_ORDER_AMBIGUOUS` gate that every read path enforces.
///
/// CLI compact is the sanctioned repair path that gate's remediation points
/// to, so it must be able to run on the ambiguous layout it repairs. Merging
/// legacy flush files epoch-first assumes the durable-coverage invariant
/// (every committed row also has a commit-domain home, enforced fail-closed
/// since #1132/#1139) — the same assumption every pre-#1138 read of a
/// non-overlapping layout already made — and the adopted output replaces all
/// inputs in the commit domain, clearing the ambiguity. Non-canonical names
/// still fail closed.
fn list_sst_files_for_adoption(dir: &Path) -> CliResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if classify_sst(&path)?.is_some() {
            files.push(path);
        }
    }
    crate::cf_read::order_sst_files(files)
}

fn durable_input_is_manifest_bounded(path: &Path, durable_seq: u64) -> bool {
    match classify_sst(path) {
        // Legacy flush files carry per-CF flush ordinals with no
        // commit-domain bound; adopting them into the durable domain is
        // exactly what CLI compact is for (issues #1132/#1138), and their
        // content is covered by the durable-coverage invariant plus the
        // seq-domain order gate in `list_sst_files`.
        Ok(Some(SstName::RouterLegacy { .. })) => true,
        Ok(Some(SstName::Flush { watermark, .. })) => watermark <= durable_seq,
        Ok(Some(SstName::DurableBatch { seq, .. } | SstName::Compacted { seq })) => {
            seq <= durable_seq
        }
        _ => false,
    }
}

fn shards_for(cf: ColumnFamily, files: &[PathBuf]) -> CliResult<Vec<SstShard>> {
    Ok(files
        .iter()
        .map(|file| SstShard::new(cf, file, 0))
        .collect::<Result<Vec<_>, _>>()?)
}

fn remove_compacted_inputs(files: &[PathBuf], report: &CompactionReport) -> CliResult {
    for file in files {
        if file != &report.output_path {
            fs::remove_file(file)
                .map_err(|error| CliError::io(format!("remove compacted input: {error}")))?;
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
