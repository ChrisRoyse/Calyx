use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use calyx_aster::vault::AsterVault;
use calyx_core::{
    AbsentReason, CxId, Input, Lens, Modality, Result, SlotId, SlotShape, SlotVector,
    TEMPORAL_MISSING_CREATED_AT,
};
use calyx_registry::{
    AlgorithmicPanelLens, BackfillConfig, BackfillPriority, BackfillRequest, BackfillScheduler,
    DecayFunction, E2RecencyConfig, E2RecencyLens, E3PeriodicConfig, E3PeriodicLens,
    E4PositionalConfig, E4PositionalLens, PanelLensRuntime, PanelSlotSpec, PeriodicOptions,
    SequenceOptions, TeiHttpLens, instantiate_panel, text_default,
};
use serde::{Deserialize, Serialize};

use super::adapter::VaultSqliteAdapter;
use super::errors;
use super::manifest::{now_ms, panel_path, scheduler_path};
use super::reader::ChunkRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackfillMode {
    RealTei,
    OfflineDeterministic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackfillSummary {
    pub panel_template: String,
    pub panel_version: u32,
    pub source_rows: usize,
    pub scheduled_slots: usize,
    pub batches_completed: usize,
    pub slot_rows_written: usize,
    pub scheduler_path: String,
}

pub fn backfill_default_panel(
    vault: &AsterVault,
    vault_dir: &Path,
    rows: &[ChunkRow],
    adapter: &VaultSqliteAdapter,
    mode: BackfillMode,
    batch_size: usize,
) -> Result<BackfillSummary> {
    let instantiated = instantiate_panel(&text_default(), now_ms());
    let panel_json = serde_json::to_vec_pretty(&instantiated.panel)
        .map_err(|err| errors::manifest(format!("encode panel: {err}")))?;
    let path = panel_path(vault_dir);
    fs::write(&path, panel_json).map_err(|err| errors::io("write panel", &path, err))?;

    let scheduler_file = scheduler_path(vault_dir);
    let mut scheduler = BackfillScheduler::open(
        &scheduler_file,
        BackfillConfig {
            max_concurrent: 1,
            batch_size: batch_size.max(1),
            throttle_ms: 0,
        },
    )?;
    let candidates = ordered_candidates(rows, adapter);
    for slot in instantiated.panel.slots.iter().skip(1) {
        scheduler.enqueue(BackfillRequest {
            slot_id: slot.slot_id,
            lens_id: slot.lens_id,
            priority: BackfillPriority::Kernel,
            candidates: candidates.clone(),
        })?;
    }

    let by_id = rows
        .iter()
        .map(|row| (adapter.cx_id(row), row))
        .collect::<BTreeMap<_, _>>();
    let positions = rows
        .iter()
        .enumerate()
        .map(|(position, row)| (adapter.cx_id(row), position as u64))
        .collect::<BTreeMap<_, _>>();
    let temporal = TemporalContext::from_rows(rows);
    let mut batches_completed = 0;
    let mut slot_rows_written = 0;
    while let Some(batch) = scheduler.claim_next_batch(now_ms())? {
        if batch.throttled {
            continue;
        }
        let slot = instantiated
            .panel
            .slots
            .iter()
            .find(|slot| slot.slot_id == batch.slot_id)
            .ok_or_else(|| errors::manifest(format!("unknown slot {}", batch.slot_id)))?;
        let spec = &instantiated.slot_specs[usize::from(slot.slot_id.get())];
        for cx_id in &batch.candidates {
            let row = by_id
                .get(cx_id)
                .ok_or_else(|| errors::backfill_incomplete(format!("{cx_id} missing row bytes")))?;
            let position = *positions
                .get(cx_id)
                .ok_or_else(|| errors::backfill_incomplete(format!("{cx_id} missing position")))?;
            let vector = measure_slot(spec, row, mode, position, temporal)?;
            vault.put_slot_vector(*cx_id, batch.slot_id, &vector)?;
            slot_rows_written += 1;
        }
        scheduler.complete_batch(batch.slot_id, batch.lens_id, now_ms())?;
        batches_completed += 1;
    }
    vault.flush()?;
    Ok(BackfillSummary {
        panel_template: instantiated.template_name,
        panel_version: instantiated.panel.version,
        source_rows: rows.len(),
        scheduled_slots: instantiated.panel.slots.len().saturating_sub(1),
        batches_completed,
        slot_rows_written,
        scheduler_path: scheduler_file.display().to_string(),
    })
}

pub fn default_slot_ids() -> Vec<SlotId> {
    instantiate_panel(&text_default(), 0)
        .panel
        .slots
        .iter()
        .map(|slot| slot.slot_id)
        .collect()
}

fn ordered_candidates(rows: &[ChunkRow], adapter: &VaultSqliteAdapter) -> Vec<CxId> {
    let mut ranked = rows
        .iter()
        .enumerate()
        .map(|(idx, row)| (priority_rank(row, idx), idx, adapter.cx_id(row)))
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(rank, idx, _)| (*rank, *idx));
    ranked.into_iter().map(|(_, _, cx_id)| cx_id).collect()
}

fn priority_rank(row: &ChunkRow, idx: usize) -> u8 {
    if idx == 0 || row.chunk_id.contains("kernel") {
        0
    } else if row.chunk_id.contains("hot") {
        1
    } else {
        2
    }
}

#[derive(Clone, Copy, Debug)]
struct TemporalContext {
    reference_time: i64,
    max_age_secs: i64,
    total_position: u64,
}

impl TemporalContext {
    fn from_rows(rows: &[ChunkRow]) -> Self {
        let mut min = i64::MAX;
        let mut max = i64::MIN;
        for timestamp in rows.iter().filter_map(|row| {
            row.event_time_secs
                .and_then(|secs| i64::try_from(secs).ok())
        }) {
            min = min.min(timestamp);
            max = max.max(timestamp);
        }
        if min == i64::MAX {
            min = 0;
            max = 0;
        }
        Self {
            reference_time: max,
            max_age_secs: max.saturating_sub(min).max(1),
            total_position: rows.len().saturating_sub(1) as u64,
        }
    }
}

fn measure_slot(
    spec: &PanelSlotSpec,
    row: &ChunkRow,
    mode: BackfillMode,
    position: u64,
    temporal: TemporalContext,
) -> Result<SlotVector> {
    match &spec.runtime {
        PanelLensRuntime::TeiHttp { endpoint } if mode == BackfillMode::RealTei => {
            measure_tei(spec, endpoint, row)
        }
        PanelLensRuntime::Algorithmic { lens } => {
            measure_algorithmic(lens.clone(), spec.output, row, position, temporal)
        }
        _ => vector_for_shape(spec.output, row, spec.name.as_bytes()),
    }
}

fn measure_tei(spec: &PanelSlotSpec, endpoint: &str, row: &ChunkRow) -> Result<SlotVector> {
    let SlotShape::Dense(dim) = spec.output else {
        return Err(errors::backfill_incomplete("TEI slot is not dense"));
    };
    let endpoint = normalize_tei_endpoint(endpoint);
    let lens = TeiHttpLens::new(&spec.name, endpoint, Modality::Text, dim)
        .with_timeout(Duration::from_secs(30))
        .with_max_batch(8);
    lens.measure(&Input::new(Modality::Text, row.content.clone()))
}

fn measure_algorithmic(
    lens: AlgorithmicPanelLens,
    shape: SlotShape,
    row: &ChunkRow,
    position: u64,
    temporal: TemporalContext,
) -> Result<SlotVector> {
    Ok(match lens {
        AlgorithmicPanelLens::ByteFeatures => sparse_keywords(shape, &row.content)?,
        AlgorithmicPanelLens::AstStyle => ast_style(&row.content),
        AlgorithmicPanelLens::SparseKeywords => sparse_keywords(shape, &row.content)?,
        AlgorithmicPanelLens::TemporalRecent => temporal_recent(row, temporal)?,
        AlgorithmicPanelLens::TemporalPeriodic => temporal_periodic(row, temporal)?,
        AlgorithmicPanelLens::TemporalPositional => temporal_positional(row, position, temporal)?,
        AlgorithmicPanelLens::Scalar => SlotVector::Dense {
            dim: 1,
            data: vec![row.content.len() as f32],
        },
    })
}

fn temporal_recent(row: &ChunkRow, context: TemporalContext) -> Result<SlotVector> {
    let Some(input) = event_time_input(row)? else {
        return Ok(temporal_absent());
    };
    E2RecencyLens::new(E2RecencyConfig {
        decay: DecayFunction::Linear {
            max_age_secs: context.max_age_secs,
        },
        reference_time: context.reference_time,
    })
    .measure(&input)
}

fn temporal_periodic(row: &ChunkRow, context: TemporalContext) -> Result<SlotVector> {
    let Some(input) = event_time_input(row)? else {
        return Ok(temporal_absent());
    };
    E3PeriodicLens::new(E3PeriodicConfig {
        options: PeriodicOptions {
            target_hour: None,
            target_day_of_week: None,
            use_now: true,
        },
        reference_time: context.reference_time,
    })
    .measure(&input)
}

fn temporal_positional(
    row: &ChunkRow,
    position: u64,
    context: TemporalContext,
) -> Result<SlotVector> {
    if row.event_time_secs.is_none() {
        return Ok(temporal_absent());
    }
    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(&position.to_le_bytes());
    bytes.extend_from_slice(&context.total_position.to_le_bytes());
    E4PositionalLens::new(E4PositionalConfig {
        options: SequenceOptions::default(),
    })
    .measure(&Input::new(Modality::Structured, bytes))
}

fn event_time_input(row: &ChunkRow) -> Result<Option<Input>> {
    let Some(secs) = row.event_time_secs else {
        return Ok(None);
    };
    let timestamp = i64::try_from(secs).map_err(|_| {
        errors::backfill_incomplete(format!(
            "row {} source event timestamp {secs} exceeds i64",
            row.row_num
        ))
    })?;
    Ok(Some(Input::new(
        Modality::Structured,
        timestamp.to_le_bytes().to_vec(),
    )))
}

fn temporal_absent() -> SlotVector {
    SlotVector::Absent {
        reason: AbsentReason::Error(TEMPORAL_MISSING_CREATED_AT.to_string()),
    }
}

fn sparse_keywords(shape: SlotShape, bytes: &[u8]) -> Result<SlotVector> {
    let SlotShape::Sparse(dim) = shape else {
        return vector_for_shape(shape, &ChunkRowStub::new(bytes), b"byte-features");
    };
    let mut counts = BTreeMap::<u32, f32>::new();
    for term in String::from_utf8_lossy(bytes).split_whitespace() {
        let digest = calyx_core::content_address([term.as_bytes()]);
        let hash = u32::from_be_bytes(digest[..4].try_into().unwrap());
        *counts.entry(hash % dim).or_insert(0.0) += 1.0;
    }
    let total = counts.values().sum::<f32>().max(1.0);
    Ok(SlotVector::Sparse {
        dim,
        entries: counts
            .into_iter()
            .map(|(idx, val)| calyx_core::SparseEntry {
                idx,
                val: val / total,
            })
            .collect(),
    })
}

fn ast_style(bytes: &[u8]) -> SlotVector {
    let text = String::from_utf8_lossy(bytes);
    let len = bytes.len().max(1) as f32;
    let count = |needle: &str| text.matches(needle).count() as f32 / len;
    SlotVector::Dense {
        dim: 8,
        data: vec![
            count("fn"),
            count("let"),
            count("struct"),
            count("impl"),
            bytes.iter().filter(|b| matches!(b, b'{' | b'}')).count() as f32 / len,
            bytes.iter().filter(|b| **b == b';').count() as f32 / len,
            bytes.iter().filter(|b| **b == b'(').count() as f32 / len,
            bytes.iter().filter(|b| **b == b'\n').count() as f32 / len,
        ],
    }
}

fn vector_for_shape<T: RowBytes>(shape: SlotShape, row: &T, seed: &[u8]) -> Result<SlotVector> {
    Ok(match shape {
        SlotShape::Dense(dim) => SlotVector::Dense {
            dim,
            data: deterministic_dense(row.bytes(), seed, dim),
        },
        SlotShape::Sparse(dim) => SlotVector::Sparse {
            dim,
            entries: Vec::new(),
        },
        SlotShape::Multi { token_dim } => SlotVector::Multi {
            token_dim,
            tokens: vec![deterministic_dense(row.bytes(), seed, token_dim)],
        },
    })
}

fn deterministic_dense(bytes: &[u8], seed: &[u8], dim: u32) -> Vec<f32> {
    let mut out = Vec::with_capacity(dim as usize);
    let mut counter = 0_u32;
    while out.len() < dim as usize {
        let counter_bytes = counter.to_be_bytes();
        let mut hasher = blake3::Hasher::new();
        hasher.update(seed);
        hasher.update(bytes);
        hasher.update(&counter_bytes);
        for chunk in hasher.finalize().as_bytes().chunks_exact(4) {
            let raw = u32::from_be_bytes(chunk.try_into().unwrap());
            out.push((raw as f32 / u32::MAX as f32) * 2.0 - 1.0);
            if out.len() == dim as usize {
                break;
            }
        }
        counter = counter.saturating_add(1);
    }
    out
}

fn normalize_tei_endpoint(endpoint: &str) -> String {
    if endpoint.ends_with("/embed") {
        endpoint.to_string()
    } else {
        format!("{}/embed", endpoint.trim_end_matches('/'))
    }
}

trait RowBytes {
    fn bytes(&self) -> &[u8];
}

impl RowBytes for ChunkRow {
    fn bytes(&self) -> &[u8] {
        &self.content
    }
}

struct ChunkRowStub<'a>(&'a [u8]);

impl<'a> ChunkRowStub<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }
}

impl RowBytes for ChunkRowStub<'_> {
    fn bytes(&self) -> &[u8] {
        self.0
    }
}
