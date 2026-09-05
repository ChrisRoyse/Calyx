use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::raw_large_corpus::{LargeCorpusRangeState, LargeCorpusReadbackReport};

pub(crate) fn check_range_request(
    request_path: &Option<String>,
    state: &LargeCorpusRangeState,
    report: &mut LargeCorpusReadbackReport,
) {
    let Some(path) = request_path else {
        report
            .parse_failures
            .push("range_state present without request_path".to_string());
        return;
    };
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
    let value = match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) => value,
        Err(err) => {
            report
                .parse_failures
                .push(format!("{} range request JSON: {err}", path_obj.display()));
            return;
        }
    };
    if value.get("method").and_then(Value::as_str) != Some("eth_getLogs") {
        report.parse_failures.push(format!(
            "{} range request method was not eth_getLogs",
            path_obj.display()
        ));
        return;
    }
    let Some(filter) = value
        .get("params")
        .and_then(Value::as_array)
        .and_then(|params| params.first())
        .and_then(Value::as_object)
    else {
        report.parse_failures.push(format!(
            "{} range request filter missing",
            path_obj.display()
        ));
        return;
    };
    check_filter_field(
        path_obj,
        filter,
        "fromBlock",
        &hex_block(state.from_block),
        report,
    );
    check_filter_field(
        path_obj,
        filter,
        "toBlock",
        &hex_block(state.to_block),
        report,
    );
    let actual_address = filter.get("address").and_then(Value::as_str);
    if !actual_address.is_some_and(|actual| actual.eq_ignore_ascii_case(&state.address)) {
        report.parse_failures.push(format!(
            "{} range address mismatch expected {} actual {:?}",
            path_obj.display(),
            state.address,
            actual_address
        ));
    }
    let actual_topics = filter
        .get("topics")
        .and_then(Value::as_array)
        .map(|topics| {
            topics
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if actual_topics != state.topics {
        report.parse_failures.push(format!(
            "{} range topics mismatch expected {:?} actual {:?}",
            path_obj.display(),
            state.topics,
            actual_topics
        ));
    }
    if state.to_block < state.from_block {
        report.parse_failures.push(format!(
            "{} range to_block before from_block",
            path_obj.display()
        ));
        return;
    }
    let actual_count = state.to_block - state.from_block + 1;
    if actual_count != state.requested_block_count {
        report.parse_failures.push(format!(
            "{} range count mismatch expected {} actual {}",
            path_obj.display(),
            state.requested_block_count,
            actual_count
        ));
    }
    match state.limit_semantics.as_str() {
        "within_chunk_limit" if actual_count > state.max_blocks_per_chunk => {
            report.parse_failures.push(format!(
                "{} range count {} exceeds chunk limit {}",
                path_obj.display(),
                actual_count,
                state.max_blocks_per_chunk
            ));
        }
        "expected_over_limit" if actual_count <= state.max_blocks_per_chunk => {
            report.parse_failures.push(format!(
                "{} over-limit edge count {} did not exceed chunk limit {}",
                path_obj.display(),
                actual_count,
                state.max_blocks_per_chunk
            ));
        }
        "within_chunk_limit" | "expected_over_limit" => {}
        other => report.parse_failures.push(format!(
            "{} unknown range limit semantics {other}",
            path_obj.display()
        )),
    }
}

fn check_filter_field(
    path: &Path,
    filter: &serde_json::Map<String, Value>,
    field: &str,
    expected: &str,
    report: &mut LargeCorpusReadbackReport,
) {
    let actual = filter.get(field).and_then(Value::as_str);
    if actual != Some(expected) {
        report.parse_failures.push(format!(
            "{} range {field} mismatch expected {expected} actual {:?}",
            path.display(),
            actual
        ));
    }
}

fn hex_block(block: u64) -> String {
    format!("0x{block:x}")
}
