use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::Duration;

use serde_json::json;

use crate::assay_corpus_build::lens::{BuildLens, measure_text_batch};
use crate::error::{CliError, CliResult};
use crate::partitioned_bench::{rrf_plan, timeline_store};

use super::args::Args;
use super::rows::{self, Row, RowStats};
use super::{io_error, local_error};

mod bits;
mod db;
mod evidence;
mod panel;
mod parallel;
mod paths;
mod pre_gate;
mod progress;
mod selection;
mod worker;

use super::format::{self, VectorFormat};
use evidence::{Evidence, LensEvidence, TEMPORAL_COUNTS_TOWARD_A35, TEMPORAL_LANE_ROLE};
use paths::{display, display_final};
use selection::{SelectedLens, selected_lenses};

struct FbinSink {
    corpus: BufWriter<File>,
    queries: BufWriter<File>,
    format: VectorFormat,
    corpus_written: usize,
    query_written: usize,
}

struct LensStream<'a> {
    args: &'a Args,
    stats: &'a RowStats,
    lens: &'a BuildLens,
    effective_batch_size: usize,
    sink: &'a mut FbinSink,
    progress: Option<&'a mut progress::ProgressLog>,
}

struct StagedExport {
    evidence: Evidence,
    progress: progress::ProgressLog,
    plan: rrf_plan::Plan,
}

pub(crate) fn run(args: &Args) -> CliResult<Evidence> {
    let mut args = args.clone();
    let lens_template = super::template::materialize_for_stream(&mut args)?;
    let lens_template_readback = lens_template.readback.clone();
    let staging = staging_dir(&args.out_dir);
    fail_if_exists(&args.out_dir)?;
    fail_if_exists(&staging)?;
    panel::validate_floor_before_runtime(&args)?;
    let pre_encode_gate = pre_gate::validate_before_full_encode(&args)?;
    let lenses = selected_lenses(&args)?;
    let stats = rows::scan(&args)?;
    create_parent(&args.out_dir)?;
    fs::create_dir(&staging).map_err(io_error)?;
    let result = run_staged(
        &args,
        &stats,
        lenses,
        &staging,
        pre_encode_gate,
        lens_template_readback,
    );
    match result {
        Ok(mut staged) => {
            if let Err(error) = fs::rename(&staging, &args.out_dir) {
                let _ = fs::remove_dir_all(&staging);
                return Err(io_error(error));
            }
            if let Err(error) = db::persist_after_promotion(&args, &mut staged) {
                let _ = fs::remove_dir_all(&args.out_dir);
                return Err(error);
            }
            if args.emit_artifacts {
                if let Err(error) = staged.progress.export_finished_after_promotion() {
                    let _ = fs::remove_dir_all(&args.out_dir);
                    return Err(error);
                }
            } else if let Err(error) = db::remove_json_artifacts(&args) {
                let _ = fs::remove_dir_all(&args.out_dir);
                return Err(error);
            }
            Ok(staged.evidence)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            if !args.emit_artifacts {
                let _ = fs::remove_dir_all(worker::worker_root(&args.out_dir));
            }
            Err(error)
        }
    }
}

pub(crate) fn run_worker(args: &Args) -> CliResult<worker::StreamWorkerReport> {
    worker::run_worker(args)
}

fn run_staged(
    args: &Args,
    stats: &RowStats,
    lenses: Vec<SelectedLens>,
    staging: &Path,
    pre_encode_gate: evidence::PreEncodeGateEvidence,
    lens_template_readback: Option<super::template::LensTemplateDbReadback>,
) -> CliResult<StagedExport> {
    let vector_dir = staging.join(args.vector_format.dir_name());
    let vault_root = staging.join("vaults");
    fs::create_dir_all(&vector_dir).map_err(io_error)?;
    fs::create_dir_all(&vault_root).map_err(io_error)?;
    let mut roster = Vec::with_capacity(lenses.len());
    let worker_root = worker::worker_root(&args.out_dir);
    fs::create_dir_all(&worker_root).map_err(io_error)?;
    let mut progress = progress::ProgressLog::create(
        &staging.join(progress::FILE_NAME),
        args,
        stats,
        lenses.len(),
        staging,
    )?;
    let reports = parallel::run_lenses(args, stats, lenses, staging, &worker_root, &mut progress)?;
    for report in reports {
        roster.push(report.into_lens_evidence(args)?);
    }
    if !args.emit_artifacts {
        fs::remove_dir_all(&worker_root).map_err(io_error)?;
    }
    let plan = write_plan(
        &staging.join("partitioned_rrf_plan.json"),
        &display_final(args, "timeline.jsonl"),
        &roster,
    )?;
    let evidence = Evidence {
        artifact_mode: if args.emit_artifacts {
            "diagnostic_files".to_string()
        } else {
            "db_only".to_string()
        },
        out_dir: display(&args.out_dir),
        rows_jsonl: display(&args.rows_jsonl),
        lens_descriptor_source: if args.lens_template_cf_root.is_some() {
            "aster_graph_cf_lens_template".to_string()
        } else {
            "diagnostic_manifest_files".to_string()
        },
        lens_template_cf_root: args
            .lens_template_cf_root
            .as_ref()
            .map(|path| display(path.as_path())),
        lens_template_key: args
            .lens_template_cf_root
            .as_ref()
            .map(|_| args.lens_template_key.clone()),
        lens_template_db_readback: lens_template_readback,
        plan_path: display_final(args, "partitioned_rrf_plan.json"),
        plan_cf_root: display_final(args, "partitioned_rrf_plan_cf"),
        plan_association_key: rrf_plan::DEFAULT_ASSOCIATION_KEY.to_string(),
        plan_db_readback: None,
        timeline_path: display_final(args, "timeline.jsonl"),
        timeline_cf_root: display_final(args, "partitioned_rrf_timeline_cf"),
        timeline_association_key: timeline_store::DEFAULT_ASSOCIATION_KEY.to_string(),
        timeline_db_readback: None,
        progress_path: display_final(args, progress::FILE_NAME),
        export_report_path: display_final(args, "stream_fbin_report.json"),
        vector_dir: display_final(args, args.vector_format.dir_name()),
        fbin_dir: (args.vector_format == VectorFormat::Fbin)
            .then(|| display_final(args, args.vector_format.dir_name())),
        vault_root: display_final(args, "vaults"),
        dataset: args.dataset.clone(),
        vector_format: args.vector_format,
        vector_storage_contract: args.vector_format.storage_contract(),
        rows: stats.clone(),
        query_count: args.query_count,
        batch_size: args.batch_size,
        min_bits: args.min_bits,
        pre_encode_gate,
        streaming: true,
        temporal_counts_toward_a35: TEMPORAL_COUNTS_TOWARD_A35,
        temporal_lane_role: TEMPORAL_LANE_ROLE,
        lens_roster: roster,
    };
    if args.emit_artifacts {
        fs::write(
            staging.join("stream_fbin_report.json"),
            serde_json::to_vec_pretty(&evidence).map_err(|error| {
                CliError::runtime(format!("serialize stream_fbin_report.json: {error}"))
            })?,
        )
        .map_err(io_error)?;
    }
    Ok(StagedExport {
        evidence,
        progress,
        plan,
    })
}

fn create_sink(
    corpus_path: &Path,
    queries_path: &Path,
    dim: usize,
    rows: usize,
    args: &Args,
) -> CliResult<FbinSink> {
    let mut corpus = BufWriter::new(File::create(corpus_path).map_err(io_error)?);
    let mut queries = BufWriter::new(File::create(queries_path).map_err(io_error)?);
    format::write_header(&mut corpus, args.vector_format, dim, rows)?;
    format::write_header(&mut queries, args.vector_format, dim, args.query_count)?;
    Ok(FbinSink {
        corpus,
        queries,
        format: args.vector_format,
        corpus_written: 0,
        query_written: 0,
    })
}

fn stream_lens(
    mut stream: LensStream<'_>,
    write_timeline: bool,
    timeline_path: &Path,
) -> CliResult {
    let mut timeline = if write_timeline {
        Some(BufWriter::new(
            File::create(timeline_path).map_err(io_error)?,
        ))
    } else {
        None
    };
    let mut texts = Vec::with_capacity(stream.effective_batch_size);
    let mut metas = Vec::with_capacity(stream.effective_batch_size);
    rows::for_each_selected(stream.args, |row_idx, row| {
        texts.push(row.text.clone());
        metas.push((row_idx, row));
        if texts.len() >= stream.effective_batch_size {
            flush_batch(&mut stream, &mut timeline, &mut texts, &mut metas)?;
        }
        Ok(())
    })?;
    flush_batch(&mut stream, &mut timeline, &mut texts, &mut metas)?;
    if stream.sink.corpus_written != stream.stats.rows
        || stream.sink.query_written != stream.args.query_count
    {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_COUNT_MISMATCH",
            format!(
                "lens {} wrote corpus={} queries={} expected corpus={} queries={}",
                stream.lens.name(),
                stream.sink.corpus_written,
                stream.sink.query_written,
                stream.stats.rows,
                stream.args.query_count
            ),
            "inspect rows-jsonl selection and rerun stream-fbin",
        ));
    }
    if let Some(writer) = timeline.as_mut() {
        writer.flush().map_err(io_error)?;
        writer.get_ref().sync_all().map_err(io_error)?;
    }
    Ok(())
}

fn flush_batch(
    stream: &mut LensStream<'_>,
    timeline: &mut Option<BufWriter<File>>,
    texts: &mut Vec<String>,
    metas: &mut Vec<(usize, Row)>,
) -> CliResult {
    if texts.is_empty() {
        return Ok(());
    }
    let last_row_idx = metas.last().map(|(row_idx, _)| *row_idx).ok_or_else(|| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_BATCH_EMPTY",
            "batch metadata is empty while text batch is non-empty",
            "fix stream-fbin batching before trusting progress or FBIN output",
        )
    })?;
    let vectors =
        measure_text_batch(stream.lens, texts, stream.effective_batch_size).map_err(|error| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_MEASURE",
                error,
                "inspect the lens runtime and source row batch",
            )
        })?;
    if vectors.len() != metas.len() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_VECTOR_COUNT_MISMATCH",
            format!(
                "lens {} returned {} vectors for {} rows",
                stream.lens.name(),
                vectors.len(),
                metas.len()
            ),
            "fix the lens runtime batch contract",
        ));
    }
    for (vector, (row_idx, row)) in vectors.iter().zip(metas.iter()) {
        validate_vector(stream.lens, vector)?;
        format::write_row(&mut stream.sink.corpus, stream.sink.format, vector)?;
        stream.sink.corpus_written += 1;
        if *row_idx < stream.args.query_count {
            format::write_row(&mut stream.sink.queries, stream.sink.format, vector)?;
            stream.sink.query_written += 1;
        }
        if let Some(writer) = timeline.as_mut() {
            write_timeline_row(writer, *row_idx, row, stream.args.query_count)?;
        }
    }
    if stream
        .progress
        .as_ref()
        .is_some_and(|progress| progress.snapshot_due(stream.sink.corpus_written))
    {
        sync_sink(stream.sink)?;
    }
    if let Some(progress) = stream.progress.as_mut() {
        progress.batch_written(
            stream.sink.corpus_written,
            stream.sink.query_written,
            last_row_idx,
        )?;
    }
    texts.clear();
    metas.clear();
    Ok(())
}

fn elapsed_ms(duration: Duration) -> CliResult<u64> {
    u64::try_from(duration.as_millis())
        .map_err(|_| CliError::usage("stream-fbin lens elapsed_ms exceeds u64"))
}

fn validate_vector(lens: &BuildLens, vector: &[f32]) -> CliResult {
    if vector.len() != lens.dim() || vector.iter().any(|value| !value.is_finite()) {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_INVALID_VECTOR",
            format!(
                "lens {} produced dim={} expected={} or non-finite value",
                lens.name(),
                vector.len(),
                lens.dim()
            ),
            "inspect the offending lens runtime before trusting streamed FBIN",
        ));
    }
    Ok(())
}

fn write_timeline_row(
    writer: &mut BufWriter<File>,
    row_idx: usize,
    row: &Row,
    query_count: usize,
) -> CliResult {
    serde_json::to_writer(
        &mut *writer,
        &json!({
            "row_idx": row_idx,
            "id": row.id,
            "source_event_time_secs": row.event_time_secs,
            "source_event_time_raw": row.event_time_raw,
            "temporal_lane_state": row.temporal_lane_state,
            "temporal_inactive_reason": row.temporal_inactive_reason,
            "source_sequence": "jsonl_line",
            "source_sequence_index": row.source_sequence_index,
            "query_row": row_idx < query_count,
        }),
    )
    .map_err(|error| CliError::runtime(format!("write timeline row {row_idx}: {error}")))?;
    writer.write_all(b"\n").map_err(io_error)
}

fn finish_sink(sink: &mut FbinSink) -> CliResult {
    sync_sink(sink)
}

fn sync_sink(sink: &mut FbinSink) -> CliResult {
    sink.corpus.flush().map_err(io_error)?;
    sink.queries.flush().map_err(io_error)?;
    sink.corpus.get_ref().sync_all().map_err(io_error)?;
    sink.queries.get_ref().sync_all().map_err(io_error)
}

fn write_plan(
    path: &Path,
    timeline_path: &str,
    lenses: &[LensEvidence],
) -> CliResult<rrf_plan::Plan> {
    let plan = rrf_plan::Plan {
        timeline: Some(PathBuf::from(timeline_path)),
        slots: lenses
            .iter()
            .map(|lens| rrf_plan::PlanSlot {
                slot: lens.slot,
                name: Some(lens.name.clone()),
                lens_id: Some(lens.lens_id.clone()),
                weights_sha256: Some(lens.weights_sha256.clone()),
                signal_kind: Some(lens.signal_kind.clone()),
                bits_about: Some(lens.bits_about),
                vault: PathBuf::from(&lens.vault_path),
                queries: PathBuf::from(&lens.queries_path),
                query_start_row: 0,
                corpus: PathBuf::from(&lens.corpus_path),
            })
            .collect(),
    };
    let slots = lenses
        .iter()
        .map(|lens| {
            json!({
                "slot": lens.slot,
                "name": lens.name,
                "lens_id": lens.lens_id,
                "weights_sha256": lens.weights_sha256,
                "signal_kind": lens.signal_kind,
                "bits_about": lens.bits_about,
                "dim": lens.dim,
                "native_dim": lens.native_dim,
                "assay_projection": lens.assay_projection,
                "vault": lens.vault_path,
                "queries": lens.queries_path,
                "corpus": lens.corpus_path,
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "timeline": timeline_path,
            "timeline_format": "calyx-assay-timeline-v1",
            "temporal_counts_toward_a35": TEMPORAL_COUNTS_TOWARD_A35,
            "temporal_lane_role": TEMPORAL_LANE_ROLE,
            "streaming_fbin_source": true,
            "slots": slots
        }))
        .map_err(|error| CliError::runtime(format!("serialize assay plan: {error}")))?,
    )
    .map_err(io_error)?;
    Ok(plan)
}

fn fail_if_exists(path: &Path) -> CliResult {
    if path.exists() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_OUTPUT_EXISTS",
            format!("{} already exists", path.display()),
            "choose a fresh immutable output directory",
        ));
    }
    Ok(())
}

fn create_parent(path: &Path) -> CliResult {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    Ok(())
}

fn staging_dir(out_dir: &Path) -> PathBuf {
    let name = out_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("assay-stream-fbin");
    out_dir.with_file_name(format!(".{name}.tmp-{}", process::id()))
}
