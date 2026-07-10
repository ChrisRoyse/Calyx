use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::Clock;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::raw_large_corpus::LargeCorpusPage;
use crate::raw_large_corpus_onchain_chunks::{
    ChunkSourceSpec, POLYGON_RPC_SAFE_LOG_CHUNK_BLOCKS, execute_post,
};
use crate::raw_large_corpus_onchain_specs::chunk_source_specs;
use crate::raw_large_corpus_types::LargeCorpusRequest;
use crate::raw_source_support::{sha256_hex, write_json};
use crate::{PolyError, Result};

pub(crate) const ONCHAIN_BACKFILL_SCHEMA_VERSION: &str = "poly.large_corpus.onchain_backfill.v1";
const ONCHAIN_BACKFILL_STATE_FILE: &str = "onchain-backfill-state.json";
const POLYGON_CONFIRMATION_DEPTH_BLOCKS: u64 = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OnchainBackfillState {
    pub(crate) schema_version: String,
    pub(crate) source_of_truth: String,
    pub(crate) status_code: String,
    pub(crate) all_order_filled_backfill_complete: bool,
    pub(crate) chain: String,
    pub(crate) chain_id: u64,
    pub(crate) latest_block: u64,
    pub(crate) confirmation_depth_blocks: u64,
    pub(crate) latest_safe_block: u64,
    pub(crate) max_blocks_per_chunk: u64,
    pub(crate) contracts: Vec<ContractBackfillState>,
    pub(crate) next_required_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ContractBackfillState {
    pub(crate) dataset: String,
    pub(crate) endpoint: String,
    pub(crate) address: String,
    pub(crate) topic: String,
    pub(crate) rpc_url: String,
    pub(crate) docs_url: String,
    pub(crate) first_code_block: u64,
    pub(crate) first_code_block_verified: bool,
    pub(crate) code_at_first_block: bool,
    pub(crate) code_before_first_block: Option<bool>,
    pub(crate) planned_from_block: u64,
    pub(crate) planned_to_block: u64,
    pub(crate) planned_block_count: u64,
    pub(crate) planned_chunk_count: u64,
    pub(crate) captured_chunk_count: usize,
    pub(crate) captured_record_count: usize,
    pub(crate) captured_block_count: u64,
    pub(crate) captured_coverage_bps: u64,
    pub(crate) captured_ranges: Vec<CapturedRangeState>,
    pub(crate) next_required_from_block: Option<u64>,
    pub(crate) coverage_complete: bool,
    pub(crate) boundary_probe_count: usize,
    pub(crate) boundary_probes: Vec<CodeProbeSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CapturedRangeState {
    pub(crate) from_block: u64,
    pub(crate) to_block: u64,
    pub(crate) record_count: usize,
    pub(crate) request_path: String,
    pub(crate) body_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CodeProbeSummary {
    pub(crate) probe_index: usize,
    pub(crate) block: u64,
    pub(crate) request_path: String,
    pub(crate) response_path: String,
    pub(crate) request_sha256: String,
    pub(crate) response_sha256: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) body_bytes: u64,
    pub(crate) code_non_empty: bool,
    pub(crate) json_rpc_error: Option<String>,
}

pub(crate) fn write_onchain_backfill_state(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    latest_block: u64,
    pages: &[LargeCorpusPage],
    clock: &dyn Clock,
) -> Result<(PathBuf, OnchainBackfillState)> {
    if latest_block <= POLYGON_CONFIRMATION_DEPTH_BLOCKS {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_HEAD_TOO_LOW",
            format!(
                "latest Polygon block {latest_block} is not above confirmation depth {POLYGON_CONFIRMATION_DEPTH_BLOCKS}"
            ),
        ));
    }
    let latest_safe_block = latest_block - POLYGON_CONFIRMATION_DEPTH_BLOCKS;
    let mut contracts = Vec::new();
    for spec in chunk_source_specs() {
        contracts.push(contract_state(
            request,
            agent,
            latest_block,
            latest_safe_block,
            pages,
            spec,
            clock,
        )?);
    }
    let complete = contracts.iter().all(|contract| contract.coverage_complete);
    let state = OnchainBackfillState {
        schema_version: ONCHAIN_BACKFILL_SCHEMA_VERSION.to_string(),
        source_of_truth: "live public Polygon JSON-RPC eth_blockNumber and eth_getCode probes plus persisted eth_getLogs chunk files".to_string(),
        status_code: if complete {
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_COMPLETE".to_string()
        } else {
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_INCOMPLETE".to_string()
        },
        all_order_filled_backfill_complete: complete,
        chain: "polygon".to_string(),
        chain_id: 137,
        latest_block,
        confirmation_depth_blocks: POLYGON_CONFIRMATION_DEPTH_BLOCKS,
        latest_safe_block,
        max_blocks_per_chunk: POLYGON_RPC_SAFE_LOG_CHUNK_BLOCKS,
        contracts,
        next_required_action: "resume chunked eth_getLogs from each contract next_required_from_block until latest_safe_block, then read back hashes and dedupe by chain_id + exchange_address + transactionHash + logIndex".to_string(),
    };
    validate_state(&state)?;
    let path = request.output_root.join(ONCHAIN_BACKFILL_STATE_FILE);
    write_json(&path, &state)?;
    Ok((path, state))
}

fn contract_state(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    latest_block: u64,
    latest_safe_block: u64,
    pages: &[LargeCorpusPage],
    spec: ChunkSourceSpec<'_>,
    clock: &dyn Clock,
) -> Result<ContractBackfillState> {
    let proof = find_first_code_block(request, agent, latest_block, &spec, clock)?;
    let captured_ranges = captured_ranges(pages, spec.dataset);
    let captured_record_count = captured_ranges.iter().map(|range| range.record_count).sum();
    let planned_block_count = latest_safe_block - proof.first_code_block + 1;
    let captured_block_count = captured_ranges
        .iter()
        .map(|range| range.to_block - range.from_block + 1)
        .sum::<u64>()
        .min(planned_block_count);
    let next_required_from_block =
        first_gap(proof.first_code_block, latest_safe_block, &captured_ranges);
    let coverage_complete = next_required_from_block.is_none();
    let captured_coverage_bps = captured_block_count
        .saturating_mul(10_000)
        .checked_div(planned_block_count)
        .unwrap_or(0);
    Ok(ContractBackfillState {
        dataset: spec.dataset.to_string(),
        endpoint: spec.endpoint.to_string(),
        address: spec.address.to_string(),
        topic: spec.topic.to_string(),
        rpc_url: spec.rpc_url.to_string(),
        docs_url: spec.docs_url.to_string(),
        first_code_block: proof.first_code_block,
        first_code_block_verified: proof.first_code_block_verified,
        code_at_first_block: proof.code_at_first_block,
        code_before_first_block: proof.code_before_first_block,
        planned_from_block: proof.first_code_block,
        planned_to_block: latest_safe_block,
        planned_block_count,
        planned_chunk_count: div_ceil(planned_block_count, POLYGON_RPC_SAFE_LOG_CHUNK_BLOCKS),
        captured_chunk_count: captured_ranges.len(),
        captured_record_count,
        captured_block_count,
        captured_coverage_bps,
        captured_ranges,
        next_required_from_block,
        coverage_complete,
        boundary_probe_count: proof.boundary_probes.len(),
        boundary_probes: proof.boundary_probes,
    })
}

struct DeploymentProof {
    first_code_block: u64,
    first_code_block_verified: bool,
    code_at_first_block: bool,
    code_before_first_block: Option<bool>,
    boundary_probes: Vec<CodeProbeSummary>,
}

fn find_first_code_block(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    latest_block: u64,
    spec: &ChunkSourceSpec<'_>,
    clock: &dyn Clock,
) -> Result<DeploymentProof> {
    let mut probes = Vec::new();
    let head_probe = capture_code_probe(request, agent, spec, latest_block, &mut probes, clock)?;
    if !head_probe.code_non_empty || head_probe.json_rpc_error.is_some() {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_NO_CODE_AT_HEAD",
            format!(
                "{} had no code at latest block {latest_block}",
                spec.address
            ),
        ));
    }
    let mut low = 0u64;
    let mut high = latest_block;
    while low < high {
        let mid = low + (high - low) / 2;
        let probe = capture_code_probe(request, agent, spec, mid, &mut probes, clock)?;
        if probe.json_rpc_error.is_some() {
            return Err(PolyError::raw_source(
                "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_CODE_PROBE_ERROR",
                format!(
                    "eth_getCode returned JSON-RPC error for {} at block {mid}",
                    spec.address
                ),
            ));
        }
        if probe.code_non_empty {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    let first_probe = capture_code_probe(request, agent, spec, low, &mut probes, clock)?;
    let before_probe = (low > 0)
        .then(|| capture_code_probe(request, agent, spec, low - 1, &mut probes, clock))
        .transpose()?;
    let before_has_code = before_probe.as_ref().map(|probe| probe.code_non_empty);
    let verified = first_probe.code_non_empty && before_has_code != Some(true);
    if !verified {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_FIRST_CODE_UNVERIFIED",
            format!("could not prove first code block for {}", spec.address),
        ));
    }
    Ok(DeploymentProof {
        first_code_block: low,
        first_code_block_verified: true,
        code_at_first_block: first_probe.code_non_empty,
        code_before_first_block: before_has_code,
        boundary_probes: probes,
    })
}

fn capture_code_probe(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    spec: &ChunkSourceSpec<'_>,
    block: u64,
    probes: &mut Vec<CodeProbeSummary>,
    clock: &dyn Clock,
) -> Result<CodeProbeSummary> {
    let index = probes.len();
    let dir = request
        .output_root
        .join("onchain-backfill-proof")
        .join(spec.dataset);
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_PROOF_DIR_FAILED",
            format!("create onchain proof dir {}: {err}", dir.display()),
        )
    })?;
    let request_path = dir.join(format!("probe-{index:06}.request.json"));
    let response_path = dir.join(format!("probe-{index:06}.response.json"));
    let request_value = json_rpc("eth_getCode", json!([spec.address, hex_block(block)]));
    let request_bytes = persist_json(&request_path, &request_value)?;
    let (status_code, response_bytes, transport_error) = execute_post(
        clock,
        agent,
        spec.rpc_url,
        &request_bytes,
        request.max_body_bytes,
        spec.dataset,
    )?;
    if let Some(error) = transport_error {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_TRANSPORT_FAILED",
            format!(
                "eth_getCode transport failed for {} at block {block}: {error}",
                spec.address
            ),
        ));
    }
    fs::write(&response_path, &response_bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_RESPONSE_WRITE_FAILED",
            format!(
                "write onchain proof response {}: {err}",
                response_path.display()
            ),
        )
    })?;
    let response_readback = fs::read(&response_path).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_RESPONSE_READBACK_FAILED",
            format!(
                "read onchain proof response {}: {err}",
                response_path.display()
            ),
        )
    })?;
    if response_readback != response_bytes {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_RESPONSE_READBACK_MISMATCH",
            format!("response readback mismatch at {}", response_path.display()),
        ));
    }
    let value = serde_json::from_slice::<Value>(&response_bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_RESPONSE_DECODE_FAILED",
            format!(
                "decode eth_getCode response {}: {err}",
                response_path.display()
            ),
        )
    })?;
    let json_rpc_error = value.get("error").map(ToString::to_string);
    let code_non_empty = value
        .get("result")
        .and_then(Value::as_str)
        .is_some_and(|code| code != "0x");
    let summary = CodeProbeSummary {
        probe_index: index,
        block,
        request_path: request_path.display().to_string(),
        response_path: response_path.display().to_string(),
        request_sha256: sha256_hex(&request_bytes),
        response_sha256: sha256_hex(&response_bytes),
        status_code,
        body_bytes: response_bytes.len() as u64,
        code_non_empty,
        json_rpc_error,
    };
    probes.push(summary.clone());
    Ok(summary)
}

fn captured_ranges(pages: &[LargeCorpusPage], dataset: &str) -> Vec<CapturedRangeState> {
    let mut ranges = pages
        .iter()
        .filter(|page| page.dataset == dataset)
        .filter_map(|page| {
            page.range_state.as_ref().map(|state| CapturedRangeState {
                from_block: state.from_block,
                to_block: state.to_block,
                record_count: page.record_count,
                request_path: page.request_path.clone().unwrap_or_default(),
                body_path: page.body_path.clone(),
            })
        })
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| (range.from_block, range.to_block));
    ranges
}

fn first_gap(
    from_block: u64,
    to_block: u64,
    captured_ranges: &[CapturedRangeState],
) -> Option<u64> {
    let mut cursor = from_block;
    for range in captured_ranges {
        if range.to_block < cursor {
            continue;
        }
        if range.from_block > cursor {
            return Some(cursor);
        }
        cursor = range.to_block.saturating_add(1);
        if cursor > to_block {
            return None;
        }
    }
    Some(cursor)
}

fn validate_state(state: &OnchainBackfillState) -> Result<()> {
    if state.contracts.is_empty() {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_EMPTY",
            "onchain backfill state had no contracts",
        ));
    }
    for contract in &state.contracts {
        if !contract.first_code_block_verified
            || !contract.code_at_first_block
            || contract.code_before_first_block == Some(true)
        {
            return Err(PolyError::raw_source(
                "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_DEPLOYMENT_UNVERIFIED",
                format!("deployment boundary not verified for {}", contract.address),
            ));
        }
        if contract.coverage_complete != contract.next_required_from_block.is_none() {
            return Err(PolyError::raw_source(
                "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_COVERAGE_CONFLICT",
                format!("coverage state conflict for {}", contract.dataset),
            ));
        }
        if contract.boundary_probe_count != contract.boundary_probes.len() {
            return Err(PolyError::raw_source(
                "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_PROBE_COUNT_MISMATCH",
                format!("probe count mismatch for {}", contract.dataset),
            ));
        }
    }
    Ok(())
}

fn persist_json(path: &Path, value: &Value) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_REQUEST_ENCODE_FAILED",
            format!("encode onchain proof request {}: {err}", path.display()),
        )
    })?;
    fs::write(path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_REQUEST_WRITE_FAILED",
            format!("write onchain proof request {}: {err}", path.display()),
        )
    })?;
    let readback = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_REQUEST_READBACK_FAILED",
            format!("read onchain proof request {}: {err}", path.display()),
        )
    })?;
    if readback != bytes {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BACKFILL_REQUEST_READBACK_MISMATCH",
            format!("request readback mismatch at {}", path.display()),
        ));
    }
    Ok(bytes)
}

fn div_ceil(value: u64, divisor: u64) -> u64 {
    if value == 0 {
        0
    } else {
        ((value - 1) / divisor) + 1
    }
}

fn json_rpc(method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    })
}

fn hex_block(block: u64) -> String {
    format!("0x{block:x}")
}
