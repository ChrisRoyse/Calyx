use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::raw_large_corpus::{LargeCorpusManifest, LargeCorpusReadbackReport};
use crate::raw_large_corpus_onchain_backfill::{
    ONCHAIN_BACKFILL_SCHEMA_VERSION, OnchainBackfillState,
};
use crate::raw_large_corpus_onchain_specs::chunk_source_specs;
use crate::raw_source_support::sha256_hex;

pub(crate) fn check_onchain_backfill_state_artifact(
    manifest: &LargeCorpusManifest,
    report: &mut LargeCorpusReadbackReport,
) {
    if !manifest_has_onchain_chunks(manifest) {
        return;
    }
    if manifest.onchain_backfill_state_path.is_empty() {
        report
            .parse_failures
            .push("onchain chunk pages present without onchain_backfill_state_path".to_string());
        return;
    }
    report.checked_file_count += 1;
    let path = Path::new(&manifest.onchain_backfill_state_path);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            report
                .missing_files
                .push(format!("{}: {err}", path.display()));
            return;
        }
    };
    let state = match serde_json::from_slice::<OnchainBackfillState>(&bytes) {
        Ok(state) => state,
        Err(err) => {
            report.parse_failures.push(format!(
                "{} onchain backfill state JSON: {err}",
                path.display()
            ));
            return;
        }
    };
    check_state_semantics(path, &state, report);
    check_probe_files(&state, report);
}

fn check_state_semantics(
    path: &Path,
    state: &OnchainBackfillState,
    report: &mut LargeCorpusReadbackReport,
) {
    if state.schema_version != ONCHAIN_BACKFILL_SCHEMA_VERSION {
        report.parse_failures.push(format!(
            "{} unexpected onchain schema {}",
            path.display(),
            state.schema_version
        ));
    }
    if state.contracts.len() != chunk_source_specs().len() {
        report.parse_failures.push(format!(
            "{} expected {} contracts actual {}",
            path.display(),
            chunk_source_specs().len(),
            state.contracts.len()
        ));
    }
    for contract in &state.contracts {
        check_contract_semantics(path, contract, report);
    }
}

fn check_contract_semantics(
    path: &Path,
    contract: &crate::raw_large_corpus_onchain_backfill::ContractBackfillState,
    report: &mut LargeCorpusReadbackReport,
) {
    if !contract.first_code_block_verified || !contract.code_at_first_block {
        report.parse_failures.push(format!(
            "{} unverified first code block for {}",
            path.display(),
            contract.dataset
        ));
    }
    if contract.code_before_first_block == Some(true) {
        report.parse_failures.push(format!(
            "{} code existed before first_code_block for {}",
            path.display(),
            contract.dataset
        ));
    }
    if !contract.coverage_complete && contract.next_required_from_block.is_none() {
        report.parse_failures.push(format!(
            "{} incomplete coverage without next_required_from_block for {}",
            path.display(),
            contract.dataset
        ));
    }
    if contract.boundary_probe_count != contract.boundary_probes.len() {
        report.parse_failures.push(format!(
            "{} probe count mismatch for {}",
            path.display(),
            contract.dataset
        ));
    }
}

fn check_probe_files(state: &OnchainBackfillState, report: &mut LargeCorpusReadbackReport) {
    for contract in &state.contracts {
        for probe in &contract.boundary_probes {
            check_probe_file(&probe.request_path, &probe.request_sha256, report);
            check_probe_file(&probe.response_path, &probe.response_sha256, report);
        }
    }
}

fn check_probe_file(path: &str, expected_sha: &str, report: &mut LargeCorpusReadbackReport) {
    report.checked_file_count += 1;
    let path_obj = Path::new(path);
    let bytes = match fs::read(path_obj) {
        Ok(bytes) => bytes,
        Err(err) => {
            report
                .missing_files
                .push(format!("{}: {err}", path_obj.display()));
            return;
        }
    };
    let actual_sha = sha256_hex(&bytes);
    if actual_sha != expected_sha {
        report.sha_mismatches.push(format!(
            "{} expected {} actual {}",
            path_obj.display(),
            expected_sha,
            actual_sha
        ));
    }
    if serde_json::from_slice::<Value>(&bytes).is_err() {
        report
            .parse_failures
            .push(format!("{} probe JSON", path_obj.display()));
    }
}

fn manifest_has_onchain_chunks(manifest: &LargeCorpusManifest) -> bool {
    manifest
        .pages
        .iter()
        .any(|page| page.source == "polygon-rpc" && page.dataset.contains("order_filled_chunked"))
}
