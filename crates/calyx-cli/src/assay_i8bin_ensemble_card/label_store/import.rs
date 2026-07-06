use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::error::{CliError, CliResult};

use super::{ImportedLabels, args::ImportArgs, error, hex_sha256, write};

pub(crate) fn run_import(raw: &[String]) -> CliResult {
    let args = ImportArgs::parse(raw)?;
    let imported = load_rows_jsonl(&args.rows_jsonl, args.target_class)?;
    let anchor_name = args
        .anchor_name
        .unwrap_or_else(|| format!("target_class_{}", args.target_class));
    let readback = write(
        &args.cf_root,
        &args.association_key,
        &anchor_name,
        args.target_class,
        &imported.source_sha256,
        &imported.label_counts,
        &imported.labels,
        args.chunk_rows,
    )
    .map_err(CliError::Calyx)?;
    println!(
        "i8bin_label_anchor_db cf_root={} association_key={} anchor={} target_class={} rows={} positives={} negatives={} chunks={} manifest_value_sha256={} chunk_value_sha256={} readback_matches={}",
        readback.cf_root,
        readback.association_key,
        anchor_name,
        args.target_class,
        readback.row_count,
        readback.positive_count,
        readback.negative_count,
        readback.chunk_count,
        readback.manifest_value_sha256,
        readback.chunk_value_sha256,
        readback.readback_matches
    );
    Ok(())
}

pub(crate) fn load_rows_jsonl(path: &Path, target_class: usize) -> CliResult<ImportedLabels> {
    let bytes = std::fs::read(path).map_err(|err| {
        CliError::Calyx(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_IMPORT_IO",
            format!("read {} failed: {err}", path.display()),
        ))
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|err| {
        CliError::Calyx(error(
            "CALYX_FSV_ASSAY_I8BIN_LABELS_IMPORT_INVALID",
            format!("{} is not utf8: {err}", path.display()),
        ))
    })?;
    let mut labels = Vec::new();
    let mut counts = BTreeMap::new();
    for (line_idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: RowJson = serde_json::from_str(line).map_err(|err| {
            CliError::Calyx(error(
                "CALYX_FSV_ASSAY_I8BIN_LABELS_IMPORT_INVALID",
                format!("{} line {line_idx}: {err}", path.display()),
            ))
        })?;
        *counts.entry(row.label.to_string()).or_insert(0) += 1;
        labels.push(row.label == target_class);
    }
    super::validate_labels(&labels).map_err(CliError::Calyx)?;
    Ok(ImportedLabels {
        labels,
        label_counts: counts,
        source_sha256: hex_sha256(&bytes),
    })
}

#[derive(Deserialize)]
struct RowJson {
    label: usize,
}
