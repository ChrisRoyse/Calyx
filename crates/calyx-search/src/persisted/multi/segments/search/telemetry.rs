use std::cell::RefCell;
use std::env;
use std::fs;
use std::time::Duration;

use calyx_sextant::index::MaxSimCudaReport;
use serde::Serialize;

use super::*;
use crate::persisted::multi::pinned::{PinnedIndexAccess, PinnedSegmentSpec};

#[derive(Clone, Debug, Serialize)]
pub(super) struct MaxSimCudaTelemetry {
    pub(crate) backend: &'static str,
    pub(crate) source_placement: &'static str,
    pub(crate) staging_placement: &'static str,
    pub(crate) compute_placement: &'static str,
    pub(crate) slot: u16,
    pub(crate) query_rows: usize,
    pub(crate) strict: bool,
    pub(crate) filtered: bool,
    pub(crate) source_rows: usize,
    pub(crate) source_tokens: usize,
    pub(crate) source_bytes: u64,
    pub(crate) candidate_rows_requested: Option<usize>,
    pub(crate) candidate_rows_resolved: Option<usize>,
    pub(crate) resident_cache_hit: Option<bool>,
    pub(crate) rows_scanned: usize,
    pub(crate) tokens_scanned: usize,
    pub(crate) bytes_scanned: u64,
    pub(crate) rows_decoded: usize,
    pub(crate) tokens_decoded: usize,
    pub(crate) bytes_decoded: u64,
    pub(crate) physical_bytes_read: u64,
    pub(crate) rows_uploaded: usize,
    pub(crate) tokens_uploaded: usize,
    pub(crate) vector_bytes_uploaded: u64,
    pub(crate) h2d_bytes: u64,
    pub(crate) slot_elapsed_us: u128,
    pub(crate) cuda_elapsed_us: u128,
}

impl MaxSimCudaTelemetry {
    fn trace_detail(&self) -> String {
        format!(
            "maxsim_backend={} maxsim_cuda_backend={} maxsim_source_placement={} maxsim_staging_placement={} maxsim_compute_placement={} maxsim_query_rows={} maxsim_cuda_strict={} maxsim_cuda_filtered={} maxsim_source_rows={} maxsim_source_tokens={} maxsim_source_bytes={} maxsim_candidate_rows_requested={} maxsim_candidate_rows_resolved={} maxsim_resident_cache_hit={} maxsim_rows_scanned={} maxsim_tokens_scanned={} maxsim_bytes_scanned={} maxsim_rows_decoded={} maxsim_tokens_decoded={} maxsim_bytes_decoded={} maxsim_physical_bytes_read={} maxsim_rows_uploaded={} maxsim_tokens_uploaded={} maxsim_vector_bytes_uploaded={} maxsim_h2d_bytes={} maxsim_slot_elapsed_us={} maxsim_cuda_elapsed_us={}",
            self.backend,
            self.backend,
            self.source_placement,
            self.staging_placement,
            self.compute_placement,
            self.query_rows,
            self.strict,
            self.filtered,
            self.source_rows,
            self.source_tokens,
            self.source_bytes,
            option_usize(self.candidate_rows_requested),
            option_usize(self.candidate_rows_resolved),
            option_bool(self.resident_cache_hit),
            self.rows_scanned,
            self.tokens_scanned,
            self.bytes_scanned,
            self.rows_decoded,
            self.tokens_decoded,
            self.bytes_decoded,
            self.physical_bytes_read,
            self.rows_uploaded,
            self.tokens_uploaded,
            self.vector_bytes_uploaded,
            self.h2d_bytes,
            self.slot_elapsed_us,
            self.cuda_elapsed_us,
        )
    }
}

thread_local! {
    static LAST_TELEMETRY: RefCell<Option<MaxSimCudaTelemetry>> = const { RefCell::new(None) };
}

pub(crate) fn take_maxsim_cuda_detail() -> Option<String> {
    LAST_TELEMETRY.with(|cell| {
        cell.borrow_mut()
            .take()
            .map(|telemetry| telemetry.trace_detail())
    })
}

pub(super) fn record(telemetry: MaxSimCudaTelemetry, gpu: &MaxSimCudaReport) -> CliResult {
    if let Ok(path) = env::var("CALYX_SEARCH_MAXSIM_CUDA_REPORT") {
        let report = PersistedMaxSimCudaReport {
            telemetry: &telemetry,
            gpu,
        };
        fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    LAST_TELEMETRY.with(|cell| cell.replace(Some(telemetry)));
    Ok(())
}

pub(super) fn record_cpu(telemetry: MaxSimCudaTelemetry) {
    LAST_TELEMETRY.with(|cell| cell.replace(Some(telemetry)));
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resident_cpu_report(
    slot: SlotId,
    requested_candidates: usize,
    selection: &crate::persisted::multi::pinned::PinnedCandidateSelection,
    access: &PinnedIndexAccess,
    specs: &[PinnedSegmentSpec],
    query_rows: usize,
    elapsed: Duration,
) -> MaxSimCudaTelemetry {
    let source_rows = specs.iter().map(|spec| spec.row_count as usize).sum();
    let source_tokens = specs.iter().map(|spec| spec.token_count as usize).sum();
    let source_bytes = source_bytes(specs);
    let candidate_bytes = resident_bytes(selection.token_count(), access.index.token_dim as usize);
    MaxSimCudaTelemetry {
        backend: "persistent-maxsim-cpu-resident-candidates-v1",
        source_placement: "cpu-ram-verified-generation",
        staging_placement: "none",
        compute_placement: "cpu-canonical-exact",
        slot: slot.get(),
        query_rows,
        strict: false,
        filtered: true,
        source_rows,
        source_tokens,
        source_bytes,
        candidate_rows_requested: Some(requested_candidates),
        candidate_rows_resolved: Some(selection.row_count()),
        resident_cache_hit: Some(access.resident_cache_hit),
        rows_scanned: if access.resident_cache_hit {
            selection.row_count()
        } else {
            access.physical_rows_scanned
        },
        tokens_scanned: if access.resident_cache_hit {
            selection.token_count()
        } else {
            access.physical_tokens_decoded
        },
        bytes_scanned: if access.resident_cache_hit {
            candidate_bytes
        } else {
            access.physical_bytes_read
        },
        rows_decoded: access.physical_rows_scanned,
        tokens_decoded: access.physical_tokens_decoded,
        bytes_decoded: resident_bytes(
            access.physical_tokens_decoded,
            access.index.token_dim as usize,
        ),
        physical_bytes_read: access.physical_bytes_read,
        rows_uploaded: 0,
        tokens_uploaded: 0,
        vector_bytes_uploaded: 0,
        h2d_bytes: 0,
        slot_elapsed_us: elapsed.as_micros(),
        cuda_elapsed_us: 0,
    }
}

#[allow(clippy::too_many_arguments)] // Records the explicit source, cache, and GPU shapes.
pub(super) fn resident_report(
    slot: SlotId,
    strict: bool,
    requested_candidates: usize,
    stream: &ResidentCandidateChunkStream,
    access: &PinnedIndexAccess,
    specs: &[PinnedSegmentSpec],
    elapsed: Duration,
    gpu: &MaxSimCudaReport,
) -> MaxSimCudaTelemetry {
    let source_rows = specs.iter().map(|spec| spec.row_count as usize).sum();
    let source_tokens = specs.iter().map(|spec| spec.token_count as usize).sum();
    let source_bytes = source_bytes(specs);
    let candidate_bytes = resident_bytes(stream.token_count(), gpu.token_dim);
    MaxSimCudaTelemetry {
        backend: "persistent-maxsim-cuda-resident-candidates-v2",
        source_placement: "cpu-ram-verified-generation",
        staging_placement: "pageable-host",
        compute_placement: "cuda:0",
        slot: slot.get(),
        query_rows: gpu.query_rows,
        strict,
        filtered: true,
        source_rows,
        source_tokens,
        source_bytes,
        candidate_rows_requested: Some(requested_candidates),
        candidate_rows_resolved: Some(stream.row_count()),
        resident_cache_hit: Some(access.resident_cache_hit),
        rows_scanned: if access.resident_cache_hit {
            stream.row_count()
        } else {
            access.physical_rows_scanned
        },
        tokens_scanned: if access.resident_cache_hit {
            stream.token_count()
        } else {
            access.physical_tokens_decoded
        },
        bytes_scanned: if access.resident_cache_hit {
            candidate_bytes
        } else {
            access.physical_bytes_read
        },
        rows_decoded: access.physical_rows_scanned,
        tokens_decoded: access.physical_tokens_decoded,
        bytes_decoded: resident_bytes(access.physical_tokens_decoded, gpu.token_dim),
        physical_bytes_read: access.physical_bytes_read,
        rows_uploaded: gpu.total_rows,
        tokens_uploaded: gpu.total_tokens,
        vector_bytes_uploaded: candidate_bytes,
        h2d_bytes: gpu.h2d_bytes,
        slot_elapsed_us: elapsed.as_micros(),
        cuda_elapsed_us: gpu.elapsed_us,
    }
}

pub(super) fn stream_report(
    slot: SlotId,
    strict: bool,
    manifest: &MultiSegmentsManifest,
    specs: &[PinnedSegmentSpec],
    elapsed: Duration,
    gpu: &MaxSimCudaReport,
) -> MaxSimCudaTelemetry {
    let bytes = source_bytes(specs);
    MaxSimCudaTelemetry {
        backend: "persistent-maxsim-cuda-segments-v2",
        source_placement: "bounded-segment-files",
        staging_placement: "pageable-host",
        compute_placement: "cuda:0",
        slot: slot.get(),
        query_rows: gpu.query_rows,
        strict,
        filtered: false,
        source_rows: manifest.row_count,
        source_tokens: manifest.token_count,
        source_bytes: bytes,
        candidate_rows_requested: None,
        candidate_rows_resolved: None,
        resident_cache_hit: None,
        rows_scanned: manifest.row_count,
        tokens_scanned: manifest.token_count,
        bytes_scanned: bytes,
        rows_decoded: manifest.row_count,
        tokens_decoded: manifest.token_count,
        bytes_decoded: resident_bytes(manifest.token_count, gpu.token_dim),
        physical_bytes_read: bytes,
        rows_uploaded: gpu.total_rows,
        tokens_uploaded: gpu.total_tokens,
        vector_bytes_uploaded: resident_bytes(gpu.total_tokens, gpu.token_dim),
        h2d_bytes: gpu.h2d_bytes,
        slot_elapsed_us: elapsed.as_micros(),
        cuda_elapsed_us: gpu.elapsed_us,
    }
}

#[derive(Serialize)]
struct PersistedMaxSimCudaReport<'a> {
    telemetry: &'a MaxSimCudaTelemetry,
    gpu: &'a MaxSimCudaReport,
}

fn source_bytes(specs: &[PinnedSegmentSpec]) -> u64 {
    specs
        .iter()
        .fold(0_u64, |total, spec| total.saturating_add(spec.byte_len))
}

fn resident_bytes(tokens: usize, token_dim: usize) -> u64 {
    (tokens as u64)
        .saturating_mul((token_dim as u64).saturating_add(1))
        .saturating_mul(4)
}

fn option_usize(value: Option<usize>) -> String {
    value.map_or_else(|| "none".to_string(), |value| value.to_string())
}

fn option_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "none",
    }
}
