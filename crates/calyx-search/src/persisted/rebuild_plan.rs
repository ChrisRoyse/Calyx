use std::collections::BTreeMap;
use std::env;

use calyx_core::{Constellation, CxId, SlotId};
use calyx_sextant::index::DiskAnnBuildBackend;

use crate::error::{CliError, CliResult};
use crate::persisted::SearchIndexManifest;

const DEFAULT_REBUILD_SLOT_MEMORY_BUDGET_BYTES: usize = 8 * 1024 * 1024 * 1024;
const DEFAULT_REBUILD_READER_LEASE_MS: u64 = 60 * 60 * 1000;
const DEFAULT_SLOT_ROW_MEMORY_ESTIMATE_BYTES: usize = 32 * 1024;
const MIN_SLOT_MEMORY_ESTIMATE_BYTES: usize = 1024 * 1024;
const DENSE_REBUILD_MEMORY_MULTIPLIER: usize = 6;
const DENSE_ROW_OVERHEAD_BYTES: usize = 1024;
const MULTI_REBUILD_MEMORY_MULTIPLIER: usize = 2;
const MULTI_ROW_OVERHEAD_BYTES: usize = 2048;
const SPARSE_ROW_MEMORY_ESTIMATE_BYTES: usize = 4096;

#[derive(Clone, Debug)]
pub(super) struct SlotBuildPlan {
    pub(super) slot: SlotId,
    pub(super) expected_ids: Vec<CxId>,
    pub(super) estimated_bytes: usize,
}

pub(super) fn validate_parallel_rebuild_config() -> CliResult {
    configured_nonzero_usize("CALYX_SEARCH_REBUILD_MAX_PARALLEL_SLOTS")?;
    configured_nonzero_usize("RAYON_NUM_THREADS")?;
    configured_nonzero_usize("CALYX_SEARCH_REBUILD_MEMORY_BUDGET_BYTES")?;
    configured_nonzero_u64("CALYX_SEARCH_REBUILD_READER_LEASE_MS")?;
    configured_diskann_build_backend()?;
    Ok(())
}

pub(super) fn configured_rebuild_reader_lease_ms() -> CliResult<u64> {
    Ok(
        configured_nonzero_u64("CALYX_SEARCH_REBUILD_READER_LEASE_MS")?
            .unwrap_or(DEFAULT_REBUILD_READER_LEASE_MS),
    )
}

pub(super) fn configured_diskann_build_backend() -> CliResult<DiskAnnBuildBackend> {
    let raw = match env::var("CALYX_SEARCH_DISKANN_BUILD_BACKEND") {
        Ok(raw) => raw,
        Err(env::VarError::NotPresent) => return Ok(DiskAnnBuildBackend::CpuVamana),
        Err(env::VarError::NotUnicode(_)) => {
            return Err(CliError::usage(
                "CALYX_SEARCH_DISKANN_BUILD_BACKEND must be valid UTF-8 when set",
            ));
        }
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CliError::usage(
            "CALYX_SEARCH_DISKANN_BUILD_BACKEND must not be empty",
        ));
    }
    trimmed.parse::<DiskAnnBuildBackend>().map_err(|err| {
        CliError::usage(format!(
            "CALYX_SEARCH_DISKANN_BUILD_BACKEND must be cpu-vamana or cuvs-cagra, got {raw:?}: {err}"
        ))
    })
}

pub(super) fn slot_build_plans(
    base_docs: &BTreeMap<CxId, Constellation>,
    previous_manifest: Option<&SearchIndexManifest>,
) -> Vec<SlotBuildPlan> {
    let mut ids_by_slot = BTreeMap::<SlotId, Vec<CxId>>::new();
    for (cx_id, cx) in base_docs {
        for slot in cx.slots.keys() {
            ids_by_slot.entry(*slot).or_default().push(*cx_id);
        }
    }
    ids_by_slot
        .into_iter()
        .map(|(slot, mut expected_ids)| {
            expected_ids.sort();
            expected_ids.dedup();
            let estimated_bytes = estimate_slot_bytes(slot, expected_ids.len(), previous_manifest);
            SlotBuildPlan {
                slot,
                expected_ids,
                estimated_bytes,
            }
        })
        .collect()
}

pub(super) fn bounded_parallel_slot_count(plans: &[SlotBuildPlan]) -> CliResult<usize> {
    if plans.is_empty() {
        return Ok(1);
    }
    let thread_limit = configured_nonzero_usize("CALYX_SEARCH_REBUILD_MAX_PARALLEL_SLOTS")?
        .or(configured_nonzero_usize("RAYON_NUM_THREADS")?)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|threads| threads.get())
                .unwrap_or(1)
        });
    let memory_budget = configured_nonzero_usize("CALYX_SEARCH_REBUILD_MEMORY_BUDGET_BYTES")?
        .unwrap_or(DEFAULT_REBUILD_SLOT_MEMORY_BUDGET_BYTES);
    let largest_slot = plans
        .iter()
        .map(|plan| plan.estimated_bytes)
        .max()
        .unwrap_or(MIN_SLOT_MEMORY_ESTIMATE_BYTES);
    let memory_limit = (memory_budget / largest_slot).max(1);
    Ok(thread_limit.min(memory_limit).max(1).min(plans.len()))
}

fn estimate_slot_bytes(
    slot: SlotId,
    expected_len: usize,
    previous_manifest: Option<&SearchIndexManifest>,
) -> usize {
    let Some(entry) = previous_manifest
        .and_then(|manifest| manifest.slots.iter().find(|entry| entry.slot == slot.get()))
    else {
        return expected_len
            .saturating_mul(DEFAULT_SLOT_ROW_MEMORY_ESTIMATE_BYTES)
            .max(MIN_SLOT_MEMORY_ESTIMATE_BYTES);
    };
    let estimate = match entry.kind.as_str() {
        "diskann" | "flat_dense" => entry
            .dim
            .map(|dim| {
                expected_len
                    .saturating_mul(dim as usize)
                    .saturating_mul(std::mem::size_of::<f32>())
                    .saturating_mul(DENSE_REBUILD_MEMORY_MULTIPLIER)
                    .saturating_add(expected_len.saturating_mul(DENSE_ROW_OVERHEAD_BYTES))
            })
            .unwrap_or_else(|| expected_len.saturating_mul(DEFAULT_SLOT_ROW_MEMORY_ESTIMATE_BYTES)),
        "multi_maxsim_segments" => entry
            .token_dim
            .zip(entry.token_count)
            .map(|(token_dim, token_count)| {
                token_count
                    .saturating_mul(token_dim as usize)
                    .saturating_mul(std::mem::size_of::<f32>())
                    .saturating_mul(MULTI_REBUILD_MEMORY_MULTIPLIER)
                    .saturating_add(expected_len.saturating_mul(MULTI_ROW_OVERHEAD_BYTES))
            })
            .unwrap_or_else(|| expected_len.saturating_mul(DEFAULT_SLOT_ROW_MEMORY_ESTIMATE_BYTES)),
        "sparse_inverted" => expected_len.saturating_mul(SPARSE_ROW_MEMORY_ESTIMATE_BYTES),
        _ => expected_len.saturating_mul(DEFAULT_SLOT_ROW_MEMORY_ESTIMATE_BYTES),
    };
    estimate.max(MIN_SLOT_MEMORY_ESTIMATE_BYTES)
}

fn configured_nonzero_usize(name: &str) -> CliResult<Option<usize>> {
    let raw = match env::var(name) {
        Ok(raw) => raw,
        Err(env::VarError::NotPresent) => return Ok(None),
        Err(env::VarError::NotUnicode(_)) => {
            return Err(CliError::usage(format!(
                "{name} must be valid UTF-8 when set"
            )));
        }
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CliError::usage(format!("{name} must not be empty")));
    }
    let parsed = trimmed.parse::<usize>().map_err(|err| {
        CliError::usage(format!(
            "{name} must be a positive integer, got {raw:?}: {err}"
        ))
    })?;
    if parsed == 0 {
        return Err(CliError::usage(format!("{name} must be >= 1")));
    }
    Ok(Some(parsed))
}

fn configured_nonzero_u64(name: &str) -> CliResult<Option<u64>> {
    let raw = match env::var(name) {
        Ok(raw) => raw,
        Err(env::VarError::NotPresent) => return Ok(None),
        Err(env::VarError::NotUnicode(_)) => {
            return Err(CliError::usage(format!(
                "{name} must be valid UTF-8 when set"
            )));
        }
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CliError::usage(format!("{name} must not be empty")));
    }
    let parsed = trimmed.parse::<u64>().map_err(|err| {
        CliError::usage(format!(
            "{name} must be a positive integer, got {raw:?}: {err}"
        ))
    })?;
    if parsed == 0 {
        return Err(CliError::usage(format!("{name} must be >= 1")));
    }
    Ok(Some(parsed))
}
