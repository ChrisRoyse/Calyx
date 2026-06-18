use serde_json::json;

use crate::assay_bits_validation::cost::LensCost;

use super::super::data::BuildRows;
use super::super::request::CorpusBuildRequest;
use super::BuildLens;

pub(super) fn emit_start(request: &CorpusBuildRequest, rows: &BuildRows, lens: &BuildLens) {
    eprintln!(
        "{}",
        json!({
            "event": "assay_corpus_build_lens_start",
            "lens": lens.name,
            "runtime": lens.runtime_name,
            "input_count": rows.rows.len(),
            "caller_batch_size": request.batch_size,
            "max_batch": lens.spec.max_batch,
            "placement": lens.placement,
            "manifest": lens.manifest,
        })
    );
}

pub(super) fn emit_finish(
    request: &CorpusBuildRequest,
    rows: &BuildRows,
    lens: &BuildLens,
    elapsed_ms: f32,
    cost: &LensCost,
) {
    eprintln!(
        "{}",
        json!({
            "event": "assay_corpus_build_lens_finish",
            "lens": lens.name,
            "runtime": lens.runtime_name,
            "input_count": rows.rows.len(),
            "caller_batch_size": request.batch_size,
            "max_batch": lens.spec.max_batch,
            "placement": cost.placement,
            "elapsed_ms": elapsed_ms,
            "ms_per_input": cost.ms_per_input,
            "vram_mb": cost.vram_mb,
            "ram_mb": cost.ram_mb,
        })
    );
}
