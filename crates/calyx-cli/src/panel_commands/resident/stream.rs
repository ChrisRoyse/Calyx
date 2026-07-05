//! Streamed binary `measure_batch` (#1002).
//!
//! Measurement is chunk-major: each resident service chunk is measured through
//! every active lens, assembled into rows, and emitted before the next chunk
//! starts. The runtime batch limit is passed down as an internal per-forward
//! cap for runtimes that support it (#1158). Server-side memory holds one chunk of
//! multi-vector payloads instead of the whole batch, and each row travels as
//! its own length-prefixed frame — a 100+ row ColBERT-heavy batch never
//! materializes as one giant in-memory response frame on either side, and the
//! client's read timeout is bounded by one chunk of measurement, not the
//! whole batch.

use super::codec::{decode_binary, encode_binary, read_frame, write_frame};
use super::dispatch::slot_measure;
use super::parallel::measure_chunk_lenses;
use super::server::ResidentService;
use super::*;

pub(super) const MEASURE_WINDOW_ENV: &str = "CALYX_PANEL_RESIDENT_MEASURE_WINDOW";
const DEFAULT_MEASURE_WINDOW_MULTIPLIER: usize = 32;

/// Measure a batch chunk-major, handing each chunk's assembled rows to
/// `emit_rows` as soon as they exist. Returns elapsed milliseconds.
pub(super) fn measure_batch_chunked(
    service: &ResidentService,
    modality: Modality,
    input_bytes: Vec<Vec<u8>>,
    runtime_batch_limit: Option<usize>,
    emit_rows: &mut dyn FnMut(Vec<ResidentMeasuredInput>) -> CliResult,
) -> CliResult<u128> {
    if matches!(runtime_batch_limit, Some(0)) {
        return Err(CalyxError::lens_unreachable(
            "resident measure_batch runtime_batch_limit must be > 0 when supplied",
        )
        .into());
    }
    let started = Instant::now();
    let input_count = input_bytes.len();
    eprintln!(
        "CALYX_PANEL_RESIDENT_RUNTIME phase=measure_batch_start process_id={} modality={:?} inputs={input_count} runtime_batch_limit={:?}",
        std::process::id(),
        modality,
        runtime_batch_limit
    );
    let inputs = input_bytes
        .into_iter()
        .map(|bytes| Input::new(modality, bytes))
        .collect::<Vec<_>>();
    let chunk_size = resident_measure_chunk_size(inputs.len(), runtime_batch_limit)?;
    let mut emitted = 0usize;
    for (chunk_index, chunk) in inputs.chunks(chunk_size).enumerate() {
        let chunk_started = Instant::now();
        let measured_by_lens = measure_chunk_lenses(service, modality, chunk, runtime_batch_limit)?;
        let mut rows = Vec::with_capacity(chunk.len());
        for (offset, input) in chunk.iter().enumerate() {
            rows.push(assemble_row(
                service,
                modality,
                &measured_by_lens,
                emitted + offset,
                offset,
                input,
            )?);
        }
        emitted += rows.len();
        eprintln!(
            "CALYX_PANEL_RESIDENT_RUNTIME phase=measure_batch_chunk_ok process_id={} chunk_index={chunk_index} chunk_rows={} emitted_rows={emitted}/{input_count} elapsed_ms={}",
            std::process::id(),
            rows.len(),
            chunk_started.elapsed().as_millis()
        );
        emit_rows(rows)?;
    }
    let elapsed_ms = started.elapsed().as_millis();
    eprintln!(
        "CALYX_PANEL_RESIDENT_RUNTIME phase=measure_batch_ok process_id={} modality={:?} inputs={input_count} elapsed_ms={elapsed_ms}",
        std::process::id(),
        modality
    );
    Ok(elapsed_ms)
}

pub(super) fn resident_measure_chunk_size(
    input_count: usize,
    runtime_batch_limit: Option<usize>,
) -> CliResult<usize> {
    let max_input = input_count.max(1);
    if let Ok(raw) = std::env::var(MEASURE_WINDOW_ENV) {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(max_input);
        }
        let parsed = raw.parse::<usize>().ok().filter(|value| *value > 0).ok_or_else(|| {
            CalyxError {
                code: "CALYX_PANEL_RESIDENT_MEASURE_WINDOW_INVALID",
                message: format!("{MEASURE_WINDOW_ENV}={raw} is not a positive integer"),
                remediation: "set CALYX_PANEL_RESIDENT_MEASURE_WINDOW to a positive integer or unset it",
            }
        })?;
        return Ok(parsed.min(max_input));
    }
    let Some(limit) = runtime_batch_limit else {
        return Ok(max_input);
    };
    Ok(limit
        .saturating_mul(DEFAULT_MEASURE_WINDOW_MULTIPLIER)
        .max(limit)
        .min(max_input)
        .max(1))
}

pub(super) fn assemble_row(
    service: &ResidentService,
    modality: Modality,
    measured_by_lens: &BTreeMap<LensId, Vec<SlotVector>>,
    input_index: usize,
    chunk_offset: usize,
    input: &Input,
) -> CliResult<ResidentMeasuredInput> {
    let mut measured = 0;
    let mut absent = 0;
    let mut slots = Vec::with_capacity(service.state.build.panel.slots.len());
    for slot in &service.state.build.panel.slots {
        let (measured_slot, vector, absent_reason) = if slot.state != SlotState::Active {
            (false, None, Some(AbsentReason::LensInactive))
        } else if slot.modality != modality {
            (false, None, Some(AbsentReason::NotApplicable))
        } else if !service.state.build.registry.contains(slot.lens_id) {
            (false, None, Some(AbsentReason::LensUnavailable))
        } else {
            let vector = measured_by_lens
                .get(&slot.lens_id)
                .and_then(|vectors| vectors.get(chunk_offset))
                .cloned()
                .ok_or_else(|| {
                    CalyxError::lens_unreachable(format!(
                        "resident measure_batch missing measured vector for lens {} input {input_index}",
                        slot.lens_id
                    ))
                })?;
            (true, Some(vector), None)
        };
        if measured_slot {
            measured += 1;
        } else {
            absent += 1;
        }
        slots.push(slot_measure(slot, measured_slot, vector, absent_reason));
    }
    Ok(ResidentMeasuredInput {
        input_index,
        input_len: input.bytes.len(),
        measured_slot_count: measured,
        absent_slot_count: absent,
        slots,
    })
}

/// Serve one binary measure_batch connection: read the request frame, then
/// stream Header, per-row, and End frames. Any pre-measurement failure or
/// mid-measurement error is written as an Err frame so the client fails
/// closed with the structured cause.
pub(super) fn serve_binary_measure_batch(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    service: &ResidentService,
) -> CliResult {
    let request = match read_frame(reader)
        .and_then(|bytes| decode_binary::<ResidentMeasureBatchBinaryRequest>(&bytes))
    {
        Ok(request) => request,
        Err(error) => {
            return write_stream_frame(
                writer,
                &ResidentMeasureBatchStreamFrame::Err {
                    code: error.code.to_string(),
                    message: error.message,
                    remediation: error.remediation.to_string(),
                },
            );
        }
    };
    if request.protocol_version != RESIDENT_BINARY_PROTOCOL_VERSION {
        return write_stream_frame(
            writer,
            &ResidentMeasureBatchStreamFrame::Err {
                code: "CALYX_PANEL_RESIDENT_PROTOCOL_MISMATCH".to_string(),
                message: format!(
                    "resident binary measure_batch protocol version {}, expected {}",
                    request.protocol_version, RESIDENT_BINARY_PROTOCOL_VERSION
                ),
                remediation: "restart the resident service from the same Calyx build as the CLI"
                    .to_string(),
            },
        );
    }
    eprintln!(
        "CALYX_PANEL_RESIDENT_RUNTIME phase=measure_batch_binary_request process_id={} protocol_version={} inputs={}",
        std::process::id(),
        request.protocol_version,
        request.inputs.len()
    );
    write_stream_frame(
        writer,
        &ResidentMeasureBatchStreamFrame::Header(ResidentMeasureBatchStreamHeader {
            protocol_version: RESIDENT_BINARY_PROTOCOL_VERSION,
            schema: MEASURE_BATCH_SCHEMA.to_string(),
            ready: true,
            process_id: std::process::id(),
            template_source: service.state.template_source.clone(),
            modality: request.modality,
            input_count: request.inputs.len(),
            runtime_batch_limit: request.runtime_batch_limit,
        }),
    )?;
    let mut row_count = 0usize;
    let mut frame_count = 1usize;
    let stream_result = measure_batch_chunked(
        service,
        request.modality,
        request.inputs,
        request.runtime_batch_limit,
        &mut |rows| {
            for row in rows {
                write_stream_frame(writer, &ResidentMeasureBatchStreamFrame::Row(Box::new(row)))?;
                row_count += 1;
                frame_count += 1;
            }
            Ok(())
        },
    );
    match stream_result {
        Ok(elapsed_ms) => {
            write_stream_frame(
                writer,
                &ResidentMeasureBatchStreamFrame::End(ResidentMeasureBatchStreamEnd {
                    row_count,
                    elapsed_ms,
                }),
            )?;
            eprintln!(
                "CALYX_PANEL_RESIDENT_RUNTIME phase=measure_batch_binary_response process_id={} protocol_version={RESIDENT_BINARY_PROTOCOL_VERSION} rows={row_count} frames={}",
                std::process::id(),
                frame_count + 1
            );
            Ok(())
        }
        Err(error) => {
            // Fail closed mid-stream: the Err frame tells the client exactly
            // why measurement stopped; already-streamed rows must be dropped.
            write_stream_frame(
                writer,
                &ResidentMeasureBatchStreamFrame::Err {
                    code: error.code().to_string(),
                    message: error.message().to_string(),
                    remediation: error.remediation().to_string(),
                },
            )
        }
    }
}

pub(super) fn write_stream_frame(
    writer: &mut dyn Write,
    frame: &ResidentMeasureBatchStreamFrame,
) -> CliResult {
    let bytes = encode_binary(frame)?;
    write_frame(writer, &bytes)?;
    Ok(())
}
