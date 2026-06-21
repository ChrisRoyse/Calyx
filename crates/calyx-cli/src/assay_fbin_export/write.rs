use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process;

use calyx_sextant::index::VEC_MAGIC;
use serde_json::json;

use crate::error::{CliError, CliResult};

use super::args::Args;
use super::data::{self, BitsLens, LensMeta, VectorScan};
use super::timeline::{self, TimelineScanBuilder};
use super::{ExportEvidence, LensEvidence, io_error, local_error};

struct FbinSink {
    corpus: BufWriter<File>,
    queries: BufWriter<File>,
    corpus_written: usize,
    query_written: usize,
}

pub(super) fn ensure_fresh_output(args: &Args) -> CliResult {
    fail_if_exists(&args.out_dir)?;
    fail_if_exists(&staging_dir(&args.out_dir))
}

pub(super) fn write_export(
    args: &Args,
    vectors_path: &Path,
    scan: &VectorScan,
    meta: &BTreeMap<String, LensMeta>,
    bits: &BTreeMap<String, BitsLens>,
    selected: &[String],
) -> CliResult<ExportEvidence> {
    let staging = staging_dir(&args.out_dir);
    ensure_fresh_output(args)?;
    create_parent(&args.out_dir)?;
    fs::create_dir(&staging).map_err(io_error)?;
    let result = write_export_staged(args, vectors_path, scan, meta, bits, selected, &staging);
    match result {
        Ok(evidence) => {
            fs::rename(&staging, &args.out_dir).map_err(io_error)?;
            Ok(evidence)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            Err(error)
        }
    }
}

fn write_export_staged(
    args: &Args,
    vectors_path: &Path,
    scan: &VectorScan,
    meta: &BTreeMap<String, LensMeta>,
    bits: &BTreeMap<String, BitsLens>,
    selected: &[String],
    staging: &Path,
) -> CliResult<ExportEvidence> {
    let fbin_dir = staging.join("fbin");
    fs::create_dir_all(&fbin_dir).map_err(io_error)?;
    fs::create_dir_all(staging.join("vaults")).map_err(io_error)?;
    let mut sinks = create_sinks(
        selected,
        &scan.lens_dims,
        &fbin_dir,
        args.query_count,
        scan.rows,
    )?;
    stream_vectors(
        vectors_path,
        selected,
        &scan.lens_dims,
        &mut sinks,
        args.query_count,
        &staging.join("timeline.jsonl"),
    )?;
    let lens_roster = finish_sinks(args, selected, &scan.lens_dims, meta, bits, sinks)?;
    write_plan(
        &staging.join("partitioned_rrf_plan.json"),
        &display_final(args, "timeline.jsonl"),
        &lens_roster,
    )?;
    let evidence = ExportEvidence {
        out_dir: display(&args.out_dir),
        vectors_path: display(vectors_path),
        plan_path: display_final(args, "partitioned_rrf_plan.json"),
        export_report_path: display_final(args, "export_report.json"),
        timeline_path: display_final(args, "timeline.jsonl"),
        vault_root: display_final(args, "vaults"),
        rows: scan.rows,
        query_count: args.query_count,
        temporal: scan.timeline.clone(),
        lens_roster,
    };
    fs::write(
        staging.join("export_report.json"),
        serde_json::to_vec_pretty(&evidence).map_err(CliError::from)?,
    )
    .map_err(io_error)?;
    Ok(evidence)
}

fn create_sinks(
    selected: &[String],
    dims: &BTreeMap<String, usize>,
    fbin_dir: &Path,
    query_count: usize,
    rows: usize,
) -> CliResult<BTreeMap<String, FbinSink>> {
    let mut sinks = BTreeMap::new();
    for (slot, name) in selected.iter().enumerate() {
        let dim = dims[name];
        let prefix = lens_prefix(slot, name);
        let mut corpus = BufWriter::new(
            File::create(fbin_dir.join(format!("{prefix}_corpus.fbin"))).map_err(io_error)?,
        );
        let mut queries = BufWriter::new(
            File::create(fbin_dir.join(format!("{prefix}_queries.fbin"))).map_err(io_error)?,
        );
        write_fbin_header(&mut corpus, dim, rows)?;
        write_fbin_header(&mut queries, dim, query_count)?;
        sinks.insert(
            name.clone(),
            FbinSink {
                corpus,
                queries,
                corpus_written: 0,
                query_written: 0,
            },
        );
    }
    Ok(sinks)
}

fn stream_vectors(
    path: &Path,
    selected: &[String],
    dims: &BTreeMap<String, usize>,
    sinks: &mut BTreeMap<String, FbinSink>,
    query_count: usize,
    timeline_path: &Path,
) -> CliResult {
    let text = fs::read_to_string(path).map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_IO",
            format!("read {} failed: {error}", path.display()),
            "inspect the corpus-build output and rerun export-fbin",
        )
    })?;
    let selected_set = selected.iter().cloned().collect::<BTreeSet<_>>();
    let mut timeline_writer = timeline::open_writer(timeline_path)?;
    let mut timeline_scan = TimelineScanBuilder::default();
    let mut row_idx = 0usize;
    for (line_idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row = data::parse_vector_row(line_idx, line)?;
        data::validate_row(line_idx, &row)?;
        data::ensure_selected_present(&row, &selected_set, line_idx)?;
        for name in selected {
            let vector = &row.lenses[name];
            validate_selected_vector(line_idx, name, vector, dims[name])?;
            write_selected_row(
                sinks.get_mut(name).expect("sink seeded"),
                vector,
                row_idx,
                query_count,
            )?;
        }
        let timeline_row = timeline::timeline_row(row_idx, &row, query_count)?;
        timeline_scan.push(&timeline_row);
        timeline::write_row(&mut timeline_writer, &timeline_row)?;
        row_idx += 1;
    }
    let streamed_timeline = timeline_scan.finish();
    timeline_writer.flush().map_err(io_error)?;
    timeline_writer.get_ref().sync_all().map_err(io_error)?;
    if row_idx == 0 || streamed_timeline.active_rows + streamed_timeline.inactive_rows != row_idx {
        return Err(local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_TEMPORAL_INVALID",
            format!(
                "timeline rows={} vector rows={row_idx}",
                streamed_timeline.active_rows + streamed_timeline.inactive_rows
            ),
            "timeline sidecar must contain one row per vector row",
        ));
    }
    Ok(())
}

fn validate_selected_vector(
    line_idx: usize,
    name: &str,
    vector: &[f32],
    expected_dim: usize,
) -> CliResult {
    if vector.len() != expected_dim || vector.iter().any(|value| !value.is_finite()) {
        return Err(local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_INVALID_VECTOR",
            format!(
                "line {line_idx} lens {name} dim={} expected={expected_dim} or non-finite",
                vector.len()
            ),
            "rerun corpus-build and inspect vectors.jsonl before exporting",
        ));
    }
    Ok(())
}

fn write_selected_row(
    sink: &mut FbinSink,
    vector: &[f32],
    row_idx: usize,
    query_count: usize,
) -> CliResult {
    write_f32_row(&mut sink.corpus, vector)?;
    sink.corpus_written += 1;
    if row_idx < query_count {
        write_f32_row(&mut sink.queries, vector)?;
        sink.query_written += 1;
    }
    Ok(())
}

fn finish_sinks(
    args: &Args,
    selected: &[String],
    dims: &BTreeMap<String, usize>,
    meta: &BTreeMap<String, LensMeta>,
    bits: &BTreeMap<String, BitsLens>,
    mut sinks: BTreeMap<String, FbinSink>,
) -> CliResult<Vec<LensEvidence>> {
    let mut out = Vec::with_capacity(selected.len());
    for (slot, name) in selected.iter().enumerate() {
        let mut sink = sinks.remove(name).expect("sink seeded");
        sink.corpus.flush().map_err(io_error)?;
        sink.queries.flush().map_err(io_error)?;
        sink.corpus.get_ref().sync_all().map_err(io_error)?;
        sink.queries.get_ref().sync_all().map_err(io_error)?;
        let prefix = lens_prefix(slot, name);
        out.push(LensEvidence {
            slot: u16::try_from(slot).map_err(|_| CliError::usage("slot exceeds u16"))?,
            name: name.clone(),
            lens_id: meta[name].lens_id.clone(),
            weights_sha256: meta[name].weights_sha256.clone(),
            signal_kind: meta[name].signal_kind.clone(),
            bits_about: bits[name].bits_about,
            dim: dims[name],
            native_dim: meta[name].native_dim,
            assay_projection: meta[name].assay_projection.clone(),
            corpus_path: display_final(args, &format!("fbin/{prefix}_corpus.fbin")),
            queries_path: display_final(args, &format!("fbin/{prefix}_queries.fbin")),
            vault_path: display_final(args, &format!("vaults/{prefix}")),
            corpus_rows_written: sink.corpus_written,
            query_rows_written: sink.query_written,
        });
    }
    Ok(out)
}

fn write_plan(path: &Path, timeline_path: &str, lenses: &[LensEvidence]) -> CliResult {
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
            "temporal_counts_toward_a35": false,
            "slots": slots
        }))
        .map_err(CliError::from)?,
    )
    .map_err(io_error)
}

fn write_fbin_header(writer: &mut BufWriter<File>, dim: usize, count: usize) -> CliResult {
    writer.write_all(&VEC_MAGIC).map_err(io_error)?;
    writer
        .write_all(
            &u32::try_from(dim)
                .map_err(|_| CliError::usage("fbin dim exceeds u32"))?
                .to_le_bytes(),
        )
        .map_err(io_error)?;
    writer
        .write_all(
            &u64::try_from(count)
                .map_err(|_| CliError::usage("fbin count exceeds u64"))?
                .to_le_bytes(),
        )
        .map_err(io_error)
}

fn write_f32_row(writer: &mut BufWriter<File>, vector: &[f32]) -> CliResult {
    for value in vector {
        writer.write_all(&value.to_le_bytes()).map_err(io_error)?;
    }
    Ok(())
}

fn fail_if_exists(path: &Path) -> CliResult {
    if path.exists() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_OUTPUT_EXISTS",
            format!("{} already exists", path.display()),
            "choose a fresh output directory for immutable FSV artifacts",
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
        .unwrap_or("assay-fbin-export");
    out_dir.with_file_name(format!(".{name}.tmp-{}", process::id()))
}

fn lens_prefix(slot: usize, name: &str) -> String {
    format!("slot_{slot:02}_{}", safe_name(name))
}

fn safe_name(name: &str) -> String {
    name.chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '_',
        })
        .collect()
}

fn display_final(args: &Args, rel: &str) -> String {
    display(&args.out_dir.join(rel))
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
