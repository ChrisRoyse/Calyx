use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::raw_large_corpus::LargeCorpusPage;
use crate::raw_large_corpus_onchain_backfill::{
    CapturedRangeState, ContractBackfillState, ONCHAIN_BACKFILL_SCHEMA_VERSION,
    OnchainBackfillState,
};
use crate::raw_large_corpus_onchain_chunks::{
    ChunkPlan, POLYGON_RPC_SAFE_LOG_CHUNK_BLOCKS, capture_chunk_page,
};
use crate::raw_large_corpus_onchain_specs::chunk_source_specs;
use crate::raw_large_corpus_types::LargeCorpusRequest;
use crate::raw_onchain_backfill_checkpoint_validate::read_previous_checkpoint;
use crate::raw_onchain_backfill_readback_scope::OnchainBackfillReadbackScope;
use crate::raw_onchain_backfill_runner_readback::{
    OnchainBackfillReadbackReport, readback_onchain_backfill_run_scoped,
};
use crate::raw_onchain_backfill_runner_types::{
    ONCHAIN_BACKFILL_RUN_CHECKPOINT_FILE, ONCHAIN_BACKFILL_RUN_PASSED,
    ONCHAIN_BACKFILL_RUN_REPORT_FILE, ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION,
    OnchainBackfillCheckpoint, OnchainBackfillContractCheckpoint, OnchainBackfillContractRun,
    OnchainBackfillRunReport, OnchainBackfillRunRequest, planned_chunk_count, sha256_file,
};
use crate::raw_source_support::{sha256_hex, write_json};
use crate::{PolyError, Result};

pub struct OnchainBackfillRunOutcome {
    pub report: OnchainBackfillRunReport,
    pub readback: OnchainBackfillReadbackReport,
}

pub fn run_onchain_backfill_once(
    request: OnchainBackfillRunRequest,
) -> Result<OnchainBackfillRunReport> {
    Ok(run_onchain_backfill_once_with_readback(request)?.report)
}

pub fn run_onchain_backfill_once_with_readback(
    request: OnchainBackfillRunRequest,
) -> Result<OnchainBackfillRunOutcome> {
    run_onchain_backfill_once_with_readback_scope(request, OnchainBackfillReadbackScope::Full)
}

pub fn run_onchain_backfill_once_with_readback_scope(
    request: OnchainBackfillRunRequest,
    readback_scope: OnchainBackfillReadbackScope,
) -> Result<OnchainBackfillRunOutcome> {
    let request = request.normalized()?;
    let chunk_blocks = request.max_blocks_per_chunk;
    let (state, state_sha) = read_input_state(&request.state_path)?;
    validate_input_state(&state)?;
    let previous = read_previous_checkpoint(&request.output_root, &state_sha, chunk_blocks)?;
    let mut ranges = ranges_by_dataset(&state, previous.as_ref())?;
    let capture_request = LargeCorpusRequest {
        output_root: request.output_root.clone(),
        timeout_secs: request.timeout_secs,
        max_body_bytes: request.max_body_bytes,
        page_size: 100,
        max_pages_per_dataset: request.max_chunks_per_contract,
        require_exhaustive: false,
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(request.timeout_secs)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut pages = Vec::new();
    let mut contract_runs = Vec::new();
    for contract in &state.contracts {
        let spec = spec_for_contract(contract)?;
        let planned_chunk_count = planned_chunk_count(
            contract.planned_from_block,
            contract.planned_to_block,
            chunk_blocks,
        );
        let dataset_ranges = ranges.entry(contract.dataset.clone()).or_default();
        let start_from_block = first_gap(
            contract.planned_from_block,
            contract.planned_to_block,
            dataset_ranges,
        );
        let mut current = start_from_block;
        let mut captured = Vec::new();
        while let Some(from_block) = current {
            if captured.len() >= request.max_chunks_per_contract {
                break;
            }
            let to_block = from_block
                .saturating_add(chunk_blocks.saturating_sub(1))
                .min(contract.planned_to_block);
            let chunk_index = absolute_chunk_index(contract, from_block, chunk_blocks)?;
            let chunk_count = usize::try_from(planned_chunk_count).map_err(|err| {
                PolyError::raw_source(
                    "POLY_ONCHAIN_BACKFILL_CHUNK_COUNT_TOO_LARGE",
                    format!(
                        "planned chunk count {} for {} does not fit usize: {err}",
                        planned_chunk_count, contract.dataset
                    ),
                )
            })?;
            let plan =
                ChunkPlan::within_limit(spec, from_block, to_block, chunk_index, chunk_count);
            let page = capture_chunk_page(&capture_request, &agent, &spec, &plan)?;
            if !page.expectation_met {
                return Err(PolyError::raw_source(
                    "POLY_ONCHAIN_BACKFILL_CHUNK_EXPECTATION_FAILED",
                    format!(
                        "chunk {} {}..{} failed expectation; inspect {}",
                        page.dataset, from_block, to_block, page.metadata_path
                    ),
                ));
            }
            let range = range_from_page(&page)?;
            dataset_ranges.push(range);
            sort_ranges(dataset_ranges);
            current = first_gap(
                contract.planned_from_block,
                contract.planned_to_block,
                dataset_ranges,
            );
            captured.push(page.clone());
            pages.push(page);
        }
        let next_required_from_block = first_gap(
            contract.planned_from_block,
            contract.planned_to_block,
            dataset_ranges,
        );
        contract_runs.push(OnchainBackfillContractRun {
            dataset: contract.dataset.clone(),
            address: contract.address.clone(),
            planned_from_block: contract.planned_from_block,
            planned_to_block: contract.planned_to_block,
            planned_chunk_count,
            start_from_block,
            chunks_captured_this_run: captured.len(),
            records_captured_this_run: captured.iter().map(|page| page.record_count).sum(),
            next_required_from_block,
            coverage_complete: next_required_from_block.is_none(),
        });
    }
    let checkpoint = build_checkpoint(&request, &state, &state_sha, &ranges)?;
    let checkpoint_path = request
        .output_root
        .join(ONCHAIN_BACKFILL_RUN_CHECKPOINT_FILE);
    write_json(&checkpoint_path, &checkpoint)?;
    let checkpoint_sha256 = sha256_file(&checkpoint_path)?;
    let report = OnchainBackfillRunReport {
        schema_version: ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION.to_string(),
        source_of_truth:
            "live public Polygon eth_getLogs chunk files plus physical checkpoint readback"
                .to_string(),
        status_code: ONCHAIN_BACKFILL_RUN_PASSED.to_string(),
        input_state_path: request.state_path.display().to_string(),
        input_state_sha256: state_sha,
        output_root: request.output_root.display().to_string(),
        checkpoint_path: checkpoint_path.display().to_string(),
        checkpoint_sha256,
        max_chunks_per_contract: request.max_chunks_per_contract,
        max_blocks_per_chunk: chunk_blocks,
        total_pages: pages.len(),
        total_records: pages.iter().map(|page| page.record_count).sum(),
        total_body_bytes: pages.iter().map(|page| page.body_bytes).sum(),
        all_order_filled_backfill_complete: checkpoint.all_order_filled_backfill_complete,
        next_required_action: checkpoint.next_required_action.clone(),
        pages,
        contracts: contract_runs,
        passed: true,
    };
    write_json(
        &request.output_root.join(ONCHAIN_BACKFILL_RUN_REPORT_FILE),
        &report,
    )?;
    let readback =
        readback_onchain_backfill_run_scoped(&request.output_root, chunk_blocks, readback_scope)?;
    if !readback.passed {
        return Err(PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_RUN_READBACK_FAILED",
            format!(
                "on-chain backfill readback failed with status {}",
                readback.status_code
            ),
        ));
    }
    Ok(OnchainBackfillRunOutcome { report, readback })
}

fn read_input_state(path: &Path) -> Result<(OnchainBackfillState, String)> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_STATE_READ_FAILED",
            format!("read on-chain backfill state {}: {err}", path.display()),
        )
    })?;
    let state = serde_json::from_slice::<OnchainBackfillState>(&bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_STATE_DECODE_FAILED",
            format!("decode on-chain backfill state {}: {err}", path.display()),
        )
    })?;
    Ok((state, sha256_hex(&bytes)))
}

fn validate_input_state(state: &OnchainBackfillState) -> Result<()> {
    if state.schema_version != ONCHAIN_BACKFILL_SCHEMA_VERSION {
        return Err(PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_STATE_SCHEMA_INVALID",
            format!(
                "unsupported on-chain backfill state schema {}",
                state.schema_version
            ),
        ));
    }
    if state.chain != "polygon" || state.chain_id != 137 {
        return Err(PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_CHAIN_INVALID",
            format!(
                "unsupported chain {} chain_id {}",
                state.chain, state.chain_id
            ),
        ));
    }
    if state.max_blocks_per_chunk != POLYGON_RPC_SAFE_LOG_CHUNK_BLOCKS {
        return Err(PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_CHUNK_POLICY_INVALID",
            format!(
                "state chunk size {} does not match runner chunk size {}",
                state.max_blocks_per_chunk, POLYGON_RPC_SAFE_LOG_CHUNK_BLOCKS
            ),
        ));
    }
    if state.contracts.is_empty() {
        return Err(PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_STATE_CONTRACTS_EMPTY",
            "on-chain backfill state has no contracts",
        ));
    }
    for contract in &state.contracts {
        validate_contract_state(contract)?;
    }
    Ok(())
}

fn validate_contract_state(contract: &ContractBackfillState) -> Result<()> {
    if !contract.first_code_block_verified
        || !contract.code_at_first_block
        || contract.code_before_first_block == Some(true)
    {
        return Err(PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_CONTRACT_DEPLOYMENT_UNVERIFIED",
            format!(
                "deployment boundary is not verified for {}",
                contract.dataset
            ),
        ));
    }
    if contract.planned_from_block > contract.planned_to_block {
        return Err(PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_CONTRACT_RANGE_INVALID",
            format!(
                "{} planned_from_block {} is after planned_to_block {}",
                contract.dataset, contract.planned_from_block, contract.planned_to_block
            ),
        ));
    }
    if contract.coverage_complete != contract.next_required_from_block.is_none() {
        return Err(PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_CONTRACT_COMPLETION_CONFLICT",
            format!("completion conflict for {}", contract.dataset),
        ));
    }
    Ok(())
}

fn ranges_by_dataset(
    state: &OnchainBackfillState,
    previous: Option<&OnchainBackfillCheckpoint>,
) -> Result<BTreeMap<String, Vec<CapturedRangeState>>> {
    let mut map = BTreeMap::new();
    for contract in &state.contracts {
        let mut ranges = if let Some(checkpoint) = previous
            && let Some(prior) = checkpoint
                .contracts
                .iter()
                .find(|prior| prior.dataset == contract.dataset)
        {
            prior.captured_ranges.clone()
        } else {
            Vec::new()
        };
        sort_ranges(&mut ranges);
        map.insert(contract.dataset.clone(), ranges);
    }
    Ok(map)
}

fn spec_for_contract(
    contract: &ContractBackfillState,
) -> Result<crate::raw_large_corpus_onchain_chunks::ChunkSourceSpec<'static>> {
    chunk_source_specs()
        .into_iter()
        .find(|spec| {
            spec.dataset == contract.dataset
                && spec.address.eq_ignore_ascii_case(&contract.address)
                && spec.topic.eq_ignore_ascii_case(&contract.topic)
        })
        .ok_or_else(|| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_CONTRACT_SPEC_MISSING",
                format!("no runner source spec matched {}", contract.dataset),
            )
        })
}

fn absolute_chunk_index(
    contract: &ContractBackfillState,
    from_block: u64,
    chunk_size: u64,
) -> Result<usize> {
    if from_block < contract.planned_from_block {
        return Err(PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_CHUNK_START_INVALID",
            format!(
                "chunk start {from_block} is before planned start {} for {}",
                contract.planned_from_block, contract.dataset
            ),
        ));
    }
    usize::try_from((from_block - contract.planned_from_block) / chunk_size).map_err(|err| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_CHUNK_INDEX_TOO_LARGE",
            format!(
                "chunk index for {} does not fit usize: {err}",
                contract.dataset
            ),
        )
    })
}

fn range_from_page(page: &LargeCorpusPage) -> Result<CapturedRangeState> {
    let range = page.range_state.as_ref().ok_or_else(|| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_PAGE_RANGE_MISSING",
            format!("captured page {} has no range state", page.metadata_path),
        )
    })?;
    Ok(CapturedRangeState {
        from_block: range.from_block,
        to_block: range.to_block,
        record_count: page.record_count,
        request_path: page.request_path.clone().unwrap_or_default(),
        body_path: page.body_path.clone(),
    })
}

fn build_checkpoint(
    request: &OnchainBackfillRunRequest,
    state: &OnchainBackfillState,
    state_sha: &str,
    ranges: &BTreeMap<String, Vec<CapturedRangeState>>,
) -> Result<OnchainBackfillCheckpoint> {
    let mut contracts = Vec::new();
    for contract in &state.contracts {
        let planned_chunk_count = planned_chunk_count(
            contract.planned_from_block,
            contract.planned_to_block,
            request.max_blocks_per_chunk,
        );
        let captured_ranges = ranges.get(&contract.dataset).cloned().unwrap_or_default();
        let next_required_from_block = first_gap(
            contract.planned_from_block,
            contract.planned_to_block,
            &captured_ranges,
        );
        let captured_record_count = captured_ranges.iter().map(|range| range.record_count).sum();
        let captured_block_count = captured_union_blocks(
            contract.planned_from_block,
            contract.planned_to_block,
            &captured_ranges,
        );
        contracts.push(OnchainBackfillContractCheckpoint {
            dataset: contract.dataset.clone(),
            address: contract.address.clone(),
            planned_from_block: contract.planned_from_block,
            planned_to_block: contract.planned_to_block,
            planned_chunk_count,
            captured_ranges,
            captured_chunk_count: ranges.get(&contract.dataset).map(Vec::len).unwrap_or(0),
            captured_record_count,
            captured_block_count,
            next_required_from_block,
            coverage_complete: next_required_from_block.is_none(),
        });
    }
    let complete = contracts.iter().all(|contract| contract.coverage_complete);
    Ok(OnchainBackfillCheckpoint {
        schema_version: ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION.to_string(),
        source_of_truth: "physical on-chain backfill source state, captured eth_getLogs chunks, and checkpoint JSON".to_string(),
        status_code: if complete {
            "POLY_ONCHAIN_BACKFILL_COMPLETE".to_string()
        } else {
            "POLY_ONCHAIN_BACKFILL_INCOMPLETE".to_string()
        },
        input_state_path: request.state_path.display().to_string(),
        input_state_sha256: state_sha.to_string(),
        chain: state.chain.clone(),
        chain_id: state.chain_id,
        latest_safe_block: state.latest_safe_block,
        max_blocks_per_chunk: request.max_blocks_per_chunk,
        contracts,
        all_order_filled_backfill_complete: complete,
        next_required_action: if complete {
            "read back checkpoint and proceed to normalized trade-history projection".to_string()
        } else {
            "rerun calyx-poly-onchain-backfill with the same output root until each contract coverage_complete=true".to_string()
        },
    })
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

fn captured_union_blocks(planned_from: u64, planned_to: u64, ranges: &[CapturedRangeState]) -> u64 {
    let mut cursor = planned_from;
    let mut total = 0u64;
    for range in ranges {
        if range.to_block < cursor || range.from_block > planned_to {
            continue;
        }
        let from = range.from_block.max(cursor).max(planned_from);
        let to = range.to_block.min(planned_to);
        if to >= from {
            total += to - from + 1;
            cursor = to.saturating_add(1);
        }
    }
    total
}

fn sort_ranges(ranges: &mut Vec<CapturedRangeState>) {
    ranges.sort_by_key(|range| (range.from_block, range.to_block));
    ranges.dedup_by(|left, right| {
        left.from_block == right.from_block
            && left.to_block == right.to_block
            && left.body_path == right.body_path
    });
}
