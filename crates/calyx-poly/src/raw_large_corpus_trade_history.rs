use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::raw_large_corpus::{
    LargeCorpusEdgeCase, LargeCorpusManifest, LargeCorpusPage, LargeCorpusReadbackReport,
};
use crate::raw_source_support::write_json;

pub(crate) const TRADE_HISTORY_STATE_SCHEMA_VERSION: &str = "poly.large_corpus.trade_history.v1";
const DATA_API_TRADES_DATASET: &str = "data_trades_large";
const DATA_API_CAP_EDGE: &str = "edge_data_trades_offset_cap_rejected";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LargeCorpusTradeHistoryState {
    pub(crate) schema_version: String,
    pub(crate) source_of_truth: String,
    pub(crate) all_trade_history_complete: bool,
    pub(crate) status_code: String,
    pub(crate) data_api_global_trades: DataApiGlobalTradesState,
    pub(crate) onchain_order_filled_logs: OnchainOrderFilledState,
    pub(crate) schema_rule: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DataApiGlobalTradesState {
    pub(crate) dataset: String,
    pub(crate) classification: String,
    pub(crate) all_time_complete: bool,
    pub(crate) page_count: usize,
    pub(crate) record_count: usize,
    pub(crate) cap_edge_name: String,
    pub(crate) cap_edge_present: bool,
    pub(crate) cap_status_code: Option<u16>,
    pub(crate) cap_body_bytes: u64,
    pub(crate) cap_body_sha256: Option<String>,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OnchainOrderFilledState {
    pub(crate) source: String,
    pub(crate) classification: String,
    pub(crate) complete_for_all_history: bool,
    pub(crate) dedupe_key_rule: String,
    pub(crate) join_key_rule: String,
    pub(crate) total_chunk_pages: usize,
    pub(crate) total_records: usize,
    pub(crate) contracts: Vec<OnchainOrderFilledContractState>,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OnchainOrderFilledContractState {
    pub(crate) dataset: String,
    pub(crate) address: String,
    pub(crate) topic: String,
    pub(crate) page_count: usize,
    pub(crate) record_count: usize,
    pub(crate) min_from_block: u64,
    pub(crate) max_to_block: u64,
    pub(crate) max_blocks_per_chunk: u64,
    pub(crate) range_state_verified: bool,
}

pub(crate) fn write_trade_history_source_state(
    root: &Path,
    pages: &[LargeCorpusPage],
    edges: &[LargeCorpusEdgeCase],
) -> Result<(PathBuf, LargeCorpusTradeHistoryState)> {
    let state = build_trade_history_source_state(pages, edges);
    let path = root.join("trade-history-source-state.json");
    write_json(&path, &state)?;
    Ok((path, state))
}

pub(crate) fn validate_trade_history_source_state(
    state: &LargeCorpusTradeHistoryState,
) -> Vec<String> {
    let mut failures = Vec::new();
    if state.schema_version != TRADE_HISTORY_STATE_SCHEMA_VERSION {
        failures.push(format!(
            "trade-history state schema mismatch {}",
            state.schema_version
        ));
    }
    if state.all_trade_history_complete {
        failures.push(
            "trade-history state claimed all-history completion without exhaustive proof"
                .to_string(),
        );
    }
    if state.data_api_global_trades.classification != "bounded_activity_window" {
        failures.push("Data API trades classification was not bounded_activity_window".to_string());
    }
    if state.data_api_global_trades.all_time_complete {
        failures.push("Data API global trades claimed all-time completion".to_string());
    }
    if !state.data_api_global_trades.cap_edge_present {
        failures.push(format!(
            "Data API cap edge {} was not present",
            state.data_api_global_trades.cap_edge_name
        ));
    }
    if !matches!(
        state.data_api_global_trades.cap_status_code,
        Some(code) if !(200..300).contains(&code)
    ) {
        failures.push("Data API cap edge did not persist a non-2xx status".to_string());
    }
    if state.onchain_order_filled_logs.contracts.is_empty() {
        failures.push("on-chain OrderFilled source state had no contracts".to_string());
    }
    if state.onchain_order_filled_logs.dedupe_key_rule.is_empty() {
        failures.push("on-chain OrderFilled dedupe rule was empty".to_string());
    }
    if state.onchain_order_filled_logs.join_key_rule.is_empty() {
        failures.push("on-chain OrderFilled join-key rule was empty".to_string());
    }
    for contract in &state.onchain_order_filled_logs.contracts {
        if contract.page_count == 0 {
            failures.push(format!("{} had no chunk pages", contract.dataset));
        }
        if !contract.range_state_verified {
            failures.push(format!("{} had unverified range state", contract.dataset));
        }
        if contract.max_to_block < contract.min_from_block {
            failures.push(format!("{} range was inverted", contract.dataset));
        }
    }
    failures
}

pub(crate) fn check_trade_history_source_state_artifact(
    manifest: &LargeCorpusManifest,
    report: &mut LargeCorpusReadbackReport,
) {
    if manifest.trade_history_state_path.is_empty() {
        if !manifest_has_trade_history_inputs(manifest) {
            return;
        }
        report
            .parse_failures
            .push("manifest missing trade_history_state_path".to_string());
        return;
    }
    report.checked_file_count += 1;
    let path = Path::new(&manifest.trade_history_state_path);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            report
                .missing_files
                .push(format!("{}: {err}", path.display()));
            return;
        }
    };
    let state = match serde_json::from_slice::<LargeCorpusTradeHistoryState>(&bytes) {
        Ok(state) => state,
        Err(err) => {
            report
                .parse_failures
                .push(format!("{} trade-history JSON: {err}", path.display()));
            return;
        }
    };
    report
        .parse_failures
        .extend(validate_trade_history_source_state(&state));
}

fn manifest_has_trade_history_inputs(manifest: &LargeCorpusManifest) -> bool {
    manifest.pages.iter().any(|page| {
        page.dataset == DATA_API_TRADES_DATASET
            || (page.source == "polygon-rpc" && page.dataset.contains("order_filled"))
    })
}

fn build_trade_history_source_state(
    pages: &[LargeCorpusPage],
    edges: &[LargeCorpusEdgeCase],
) -> LargeCorpusTradeHistoryState {
    let data_pages = pages
        .iter()
        .filter(|page| page.dataset == DATA_API_TRADES_DATASET)
        .collect::<Vec<_>>();
    let cap_edge = edges.iter().find(|edge| edge.name == DATA_API_CAP_EDGE);
    let onchain_contracts = onchain_contract_states(pages);
    let total_chunk_pages = onchain_contracts
        .iter()
        .map(|contract| contract.page_count)
        .sum();
    let total_records = onchain_contracts
        .iter()
        .map(|contract| contract.record_count)
        .sum();
    LargeCorpusTradeHistoryState {
        schema_version: TRADE_HISTORY_STATE_SCHEMA_VERSION.to_string(),
        source_of_truth:
            "Data API /trades cap edge plus Polygon CTF/Neg Risk Exchange V2 OrderFilled log chunks persisted under this artifact root"
                .to_string(),
        all_trade_history_complete: false,
        status_code: "POLY_LARGE_CORPUS_TRADE_HISTORY_BOUNDED".to_string(),
        data_api_global_trades: DataApiGlobalTradesState {
            dataset: DATA_API_TRADES_DATASET.to_string(),
            classification: "bounded_activity_window".to_string(),
            all_time_complete: false,
            page_count: data_pages.len(),
            record_count: data_pages.iter().map(|page| page.record_count).sum(),
            cap_edge_name: DATA_API_CAP_EDGE.to_string(),
            cap_edge_present: cap_edge.is_some(),
            cap_status_code: cap_edge.and_then(|edge| edge.status_code),
            cap_body_bytes: cap_edge.map(|edge| edge.after.body_bytes).unwrap_or(0),
            cap_body_sha256: cap_edge.and_then(|edge| edge.body_sha256.clone()),
            reason:
                "live global offset cap proves this endpoint is not an all-time trade-history source"
                    .to_string(),
        },
        onchain_order_filled_logs: OnchainOrderFilledState {
            source: "polygon-rpc".to_string(),
            classification: "durable_public_order_filled_source_candidate".to_string(),
            complete_for_all_history: false,
            dedupe_key_rule:
                "dedupe by chain_id + exchange_address + transactionHash + logIndex".to_string(),
            join_key_rule:
                "join logs to market metadata through makerAssetId/takerAssetId token IDs and condition IDs from Gamma/CLOB metadata"
                    .to_string(),
            total_chunk_pages,
            total_records,
            contracts: onchain_contracts,
            reason:
                "current capture persists recent block chunks; all-history completion requires deployment-to-latest range backfill with dedupe/readback"
                    .to_string(),
        },
        schema_rule:
            "Do not derive a complete trade table from Data API /trades; use on-chain OrderFilled logs as the durable public trade source and keep completion state explicit."
                .to_string(),
    }
}

fn onchain_contract_states(pages: &[LargeCorpusPage]) -> Vec<OnchainOrderFilledContractState> {
    let mut states = Vec::new();
    for dataset in [
        "polygon_rpc_ctf_exchange_v2_order_filled_chunked_large",
        "polygon_rpc_neg_risk_exchange_v2_order_filled_chunked_large",
    ] {
        let contract_pages = pages
            .iter()
            .filter(|page| page.dataset == dataset)
            .collect::<Vec<_>>();
        let Some(first_range) = contract_pages
            .iter()
            .find_map(|page| page.range_state.as_ref())
        else {
            continue;
        };
        let min_from_block = contract_pages
            .iter()
            .filter_map(|page| page.range_state.as_ref().map(|state| state.from_block))
            .min()
            .unwrap_or(0);
        let max_to_block = contract_pages
            .iter()
            .filter_map(|page| page.range_state.as_ref().map(|state| state.to_block))
            .max()
            .unwrap_or(0);
        states.push(OnchainOrderFilledContractState {
            dataset: dataset.to_string(),
            address: first_range.address.clone(),
            topic: first_range.topics.first().cloned().unwrap_or_default(),
            page_count: contract_pages.len(),
            record_count: contract_pages.iter().map(|page| page.record_count).sum(),
            min_from_block,
            max_to_block,
            max_blocks_per_chunk: first_range.max_blocks_per_chunk,
            range_state_verified: contract_pages.iter().all(|page| {
                page.range_state.is_some()
                    && page.request_path.is_some()
                    && page.request_body_sha256.is_some()
            }),
        });
    }
    states
}
