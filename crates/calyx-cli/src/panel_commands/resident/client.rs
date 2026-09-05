use super::codec::{decode_binary, encode_binary, read_frame, write_frame};
use super::*;
use sha2::{Digest, Sha256};

pub(crate) fn client_command(args: &[String], op: &str) -> CliResult {
    let flags = parse_client_flags(args, op)?;
    if op == "measure-batch" {
        let modality = flags.modality.expect("parsed modality");
        let inputs = flags
            .inputs
            .into_iter()
            .map(|input| client_input_to_core(input, modality))
            .collect::<CliResult<Vec<_>>>()?;
        if flags.summary_only {
            let response =
                measure_batch_summary_at(flags.addr, modality, &inputs, flags.runtime_batch_limit)?;
            if let Some(path) = flags.out {
                write_json_file(path, &response)?;
            }
            return print_json(&response);
        }
        let response = measure_batch_at(flags.addr, modality, &inputs, flags.runtime_batch_limit)?;
        if let Some(path) = flags.out {
            write_json_file(path, &response.response)?;
        }
        return print_json(&response.response);
    }
    let mut request = json!({ "op": op });
    if op == "measure" {
        request["modality"] = serde_json::to_value(flags.modality.expect("parsed modality"))
            .map_err(|error| {
                CliError::runtime(format!("serialize resident measure modality: {error}"))
            })?;
        match flags.input.expect("parsed input") {
            ClientMeasureInput::Utf8(input) => request["input"] = json!(input),
            ClientMeasureInput::Hex(input_hex) => request["input_hex"] = json!(input_hex),
        }
    }
    let response = send_request(flags.addr, request)?;
    if let Some(path) = flags.out {
        write_json_file(path, &response)?;
    }
    print_json(&response)
}

fn client_input_to_core(input: ClientMeasureInput, modality: Modality) -> CliResult<Input> {
    let bytes = match input {
        ClientMeasureInput::Utf8(input) => input.into_bytes(),
        ClientMeasureInput::Hex(input_hex) => hex_decode(&input_hex).map_err(CliError::usage)?,
    };
    Ok(Input::new(modality, bytes))
}

/// Programmatic readiness probe used by ingest resident-route discovery: one
/// JSON `ready` round-trip returning the raw readiness value.
pub(crate) fn ready_value_at(addr: SocketAddr) -> CliResult<Value> {
    send_request(addr, json!({ "op": "ready" }))
}

pub(crate) fn kernel_rank_at(
    addr: SocketAddr,
    manifest_sha256: &str,
    kernel_id: CxId,
    index_sha256: &str,
    query: &TypedPanelVectors,
) -> CliResult<KernelRankAtResponse> {
    ensure_loopback(addr)?;
    let mut stream = TcpStream::connect(addr).map_err(|error| {
        CliError::from(CalyxError {
            code: "CALYX_KERNEL_RANK_UNAVAILABLE",
            message: format!("connect kernel rank resident {addr}: {error}"),
            remediation: CLIENT_TIMEOUT_REMEDIATION,
        })
    })?;
    let timeout = Some(client_timeout()?);
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    stream.write_all(KERNEL_RANK_BINARY_MAGIC)?;
    let request = encode_binary(&ResidentKernelRankBinaryRequest {
        protocol_version: KERNEL_RANK_BINARY_PROTOCOL_VERSION,
        manifest_sha256: manifest_sha256.to_string(),
        kernel_id,
        index_sha256: index_sha256.to_string(),
        query: query.clone(),
    })?;
    write_frame(&mut stream, &request)?;
    stream.flush()?;
    let response = decode_binary::<ResidentKernelRankBinaryResponse>(&read_frame(&mut stream)?)?;
    match response {
        ResidentKernelRankBinaryResponse::Err {
            code,
            message,
            remediation,
        } => Err(remote_stream_error(&code, &message, &remediation)),
        ResidentKernelRankBinaryResponse::Ok {
            protocol_version,
            process_id,
            manifest_sha256: observed_manifest,
            kernel_id: observed_kernel,
            index_sha256: observed_index,
            index_rows,
            elapsed_ms,
            nearest_json,
            ranked_ids,
        } => {
            if nearest_json.len() > MAX_KERNEL_RANK_EVIDENCE_JSON_BYTES {
                return Err(CliError::from(CalyxError {
                    code: "CALYX_KERNEL_RANK_EVIDENCE_OVERSIZE",
                    message: format!(
                        "resident {addr} returned {} evidence bytes, above the {} byte protocol bound",
                        nearest_json.len(),
                        MAX_KERNEL_RANK_EVIDENCE_JSON_BYTES
                    ),
                    remediation: "restart the resident and CLI from a matching build with bounded rank evidence",
                }));
            }
            let nearest =
                serde_json::from_slice::<TypedPanelFusionHit>(&nearest_json).map_err(|error| {
                    CliError::from(CalyxError {
                        code: "CALYX_KERNEL_RANK_EVIDENCE_DECODE",
                        message: format!(
                            "resident {addr} returned invalid nearest evidence JSON: {error}"
                        ),
                        remediation:
                            "restart the resident and CLI from a matching build with valid rank evidence",
                    })
                })?;
            let unique = ranked_ids
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            if protocol_version != KERNEL_RANK_BINARY_PROTOCOL_VERSION
                || observed_manifest != manifest_sha256
                || observed_kernel != kernel_id
                || observed_index != index_sha256
                || index_rows == 0
                || ranked_ids.len() != index_rows
                || unique.len() != ranked_ids.len()
                || ranked_ids.first().copied() != Some(nearest.cx_id)
                || nearest.lanes.is_empty()
                || !nearest.score.is_finite()
                || !(0.0..=1.0).contains(&nearest.score)
            {
                return Err(CliError::from(CalyxError {
                    code: "CALYX_KERNEL_RANK_PROTOCOL_MISMATCH",
                    message: format!(
                        "resident {addr} returned invalid rank proof protocol={protocol_version} manifest={observed_manifest} kernel={observed_kernel} index={observed_index} rows={index_rows} ranked={} unique={}",
                        ranked_ids.len(),
                        unique.len()
                    ),
                    remediation: "restart the resident from the same Calyx build and CURRENT vault generation",
                }));
            }
            Ok(KernelRankAtResponse {
                process_id,
                index_rows,
                elapsed_ms,
                nearest,
                ranked_ids,
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn search_at(
    addr: SocketAddr,
    template_source: &str,
    generation: &PersistedSearchGeneration,
    query_vectors: &[(SlotId, SlotVector)],
    metadata_query: Option<&str>,
    k: usize,
    fusion: FusionChoice,
    guard: GuardChoice,
    guard_panel_version: Option<u64>,
    filter: Option<&str>,
    explain: bool,
    freshness: SearchFreshness,
) -> CliResult<ResidentSearchAtResponse> {
    ensure_loopback(addr)?;
    let mut stream = TcpStream::connect(addr).map_err(|error| {
        CliError::from(CalyxError {
            code: "CALYX_RESIDENT_SEARCH_UNAVAILABLE",
            message: format!("connect resident search service {addr}: {error}"),
            remediation: CLIENT_TIMEOUT_REMEDIATION,
        })
    })?;
    let timeout = Some(client_timeout()?);
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    stream.write_all(SEARCH_BINARY_MAGIC)?;
    let request = encode_binary(&ResidentSearchBinaryRequest {
        protocol_version: SEARCH_BINARY_PROTOCOL_VERSION,
        template_source: template_source.to_string(),
        manifest_sha256: generation.manifest_sha256.clone(),
        base_seq: generation.base_seq,
        query_vectors: query_vectors.to_vec(),
        metadata_query: metadata_query.map(str::to_string),
        k,
        fusion,
        guard,
        guard_panel_version,
        filter: filter.map(str::to_string),
        explain,
        freshness,
    })?;
    write_frame(&mut stream, &request)?;
    stream.flush()?;
    let response_frame = read_frame(&mut stream)?;
    let response = decode_binary::<ResidentSearchBinaryResponse>(&response_frame)?;
    match response {
        ResidentSearchBinaryResponse::Err {
            code,
            message,
            remediation,
        } => Err(remote_stream_error(&code, &message, &remediation)),
        ResidentSearchBinaryResponse::Ok {
            protocol_version,
            process_id,
            template_source: observed_template,
            manifest_sha256,
            base_seq,
            elapsed_ms,
            outcome_json,
            traces,
            cache,
        } => {
            if protocol_version != SEARCH_BINARY_PROTOCOL_VERSION
                || observed_template != template_source
                || manifest_sha256 != generation.manifest_sha256
                || base_seq != generation.base_seq
            {
                return Err(CliError::from(CalyxError {
                    code: "CALYX_RESIDENT_SEARCH_PROTOCOL_MISMATCH",
                    message: format!(
                        "resident {addr} returned protocol={protocol_version} template={observed_template:?} manifest={manifest_sha256} base_seq={base_seq}; expected protocol={} template={template_source:?} manifest={} base_seq={}",
                        SEARCH_BINARY_PROTOCOL_VERSION,
                        generation.manifest_sha256,
                        generation.base_seq
                    ),
                    remediation: "restart the resident and CLI from the same Calyx build and current vault generation",
                }));
            }
            if outcome_json.len() > MAX_RESIDENT_SEARCH_OUTCOME_JSON_BYTES {
                return Err(CliError::from(CalyxError {
                    code: "CALYX_RESIDENT_SEARCH_OUTCOME_OVERSIZE",
                    message: format!(
                        "resident {addr} returned {} outcome bytes, above the {} byte protocol bound",
                        outcome_json.len(),
                        MAX_RESIDENT_SEARCH_OUTCOME_JSON_BYTES
                    ),
                    remediation: "restart the resident and CLI from a matching build with bounded search output",
                }));
            }
            let outcome =
                serde_json::from_slice::<SearchOutcome>(&outcome_json).map_err(|error| {
                    CliError::from(CalyxError {
                        code: "CALYX_RESIDENT_SEARCH_OUTCOME_DECODE",
                        message: format!(
                            "resident {addr} returned invalid search outcome JSON: {error}"
                        ),
                        remediation: "restart the resident and CLI from the same Calyx build",
                    })
                })?;
            let outcome_generation = outcome.generation.as_ref().ok_or_else(|| {
                CliError::from(CalyxError {
                    code: "CALYX_RESIDENT_SEARCH_PROTOCOL_MISMATCH",
                    message: format!("resident {addr} outcome omitted its search generation"),
                    remediation: "restart the resident and CLI from the same Calyx build",
                })
            })?;
            if outcome_generation != generation {
                return Err(CliError::from(CalyxError {
                    code: "CALYX_RESIDENT_SEARCH_GENERATION_MISMATCH",
                    message: format!(
                        "resident {addr} outcome generation differs from requested manifest={} base_seq={}",
                        generation.manifest_sha256, generation.base_seq
                    ),
                    remediation: "restart the resident against the current vault generation",
                }));
            }
            Ok(ResidentSearchAtResponse {
                process_id,
                elapsed_ms,
                outcome,
                traces,
                cache,
                request_bytes: request.len(),
                response_bytes: response_frame.len(),
            })
        }
    }
}

fn send_request(addr: SocketAddr, request: Value) -> CliResult<Value> {
    ensure_loopback(addr)?;
    let mut stream = TcpStream::connect(addr).map_err(|error| {
        CliError::from(CalyxError {
            code: "CALYX_PANEL_RESIDENT_UNAVAILABLE",
            message: format!("connect resident service {addr}: {error}"),
            remediation: CLIENT_TIMEOUT_REMEDIATION,
        })
    })?;
    let timeout = Some(client_timeout()?);
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    serde_json::to_writer(&mut stream, &request)
        .map_err(|error| CliError::runtime(format!("write resident request to {addr}: {error}")))?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    decode_json_response(addr, &response)
}

pub(super) fn decode_json_response(addr: SocketAddr, response: &str) -> CliResult<Value> {
    let value: Value = serde_json::from_str(response).map_err(|error| {
        CliError::runtime(format!("parse resident response from {addr}: {error}"))
    })?;
    match value.get("ok") {
        Some(Value::Bool(false)) => {
            let field = |name: &str| {
                value.get(name).and_then(Value::as_str).ok_or_else(|| {
                    CliError::from(CalyxError {
                        code: "CALYX_PANEL_RESIDENT_PROTOCOL_MISMATCH",
                        message: format!(
                            "resident {addr} returned ok=false without string field {name:?}: {value}"
                        ),
                        remediation:
                            "restart the resident service from the same Calyx build as the CLI",
                    })
                })
            };
            Err(remote_stream_error(
                field("code")?,
                field("message")?,
                field("remediation")?,
            ))
        }
        Some(Value::Bool(true)) | None => Ok(value),
        Some(other) => Err(CliError::from(CalyxError {
            code: "CALYX_PANEL_RESIDENT_PROTOCOL_MISMATCH",
            message: format!(
                "resident {addr} returned non-boolean ok field {other} in response: {value}"
            ),
            remediation: "restart the resident service from the same Calyx build as the CLI",
        })),
    }
}

pub(crate) fn measure_batch_at(
    addr: SocketAddr,
    modality: Modality,
    inputs: &[Input],
    runtime_batch_limit: Option<usize>,
) -> CliResult<MeasureBatchAtResponse> {
    ensure_loopback(addr)?;
    let mut stream = TcpStream::connect(addr).map_err(|error| {
        CliError::from(CalyxError {
            code: "CALYX_PANEL_RESIDENT_UNAVAILABLE",
            message: format!("connect resident service {addr}: {error}"),
            remediation: CLIENT_TIMEOUT_REMEDIATION,
        })
    })?;
    let timeout = Some(client_timeout()?);
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    stream.write_all(RESIDENT_BINARY_MAGIC)?;
    let request_bytes = encode_binary(&ResidentMeasureBatchBinaryRequest {
        protocol_version: RESIDENT_BINARY_PROTOCOL_VERSION,
        modality,
        inputs: super::input_wire::encode_inputs(modality, inputs)?,
        runtime_batch_limit,
    })?;
    write_frame(&mut stream, &request_bytes)?;
    stream.flush()?;
    read_measure_batch_stream(&mut stream, inputs.len(), request_bytes.len())
}

fn measure_batch_summary_at(
    addr: SocketAddr,
    modality: Modality,
    inputs: &[Input],
    runtime_batch_limit: Option<usize>,
) -> CliResult<MeasureBatchSummaryResponse> {
    ensure_loopback(addr)?;
    let mut stream = TcpStream::connect(addr).map_err(|error| {
        CliError::from(CalyxError {
            code: "CALYX_PANEL_RESIDENT_UNAVAILABLE",
            message: format!("connect resident service {addr}: {error}"),
            remediation: CLIENT_TIMEOUT_REMEDIATION,
        })
    })?;
    let timeout = Some(client_timeout()?);
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    stream.write_all(RESIDENT_BINARY_MAGIC)?;
    let request_bytes = encode_binary(&ResidentMeasureBatchBinaryRequest {
        protocol_version: RESIDENT_BINARY_PROTOCOL_VERSION,
        modality,
        inputs: super::input_wire::encode_inputs(modality, inputs)?,
        runtime_batch_limit,
    })?;
    write_frame(&mut stream, &request_bytes)?;
    stream.flush()?;
    read_measure_batch_summary_stream(&mut stream, inputs.len(), request_bytes.len())
}

/// Consume the streamed measure_batch frames: Header, then one Row frame per
/// input, then End. Any Err frame, out-of-order frame, truncated stream, or
/// row/count mismatch fails closed — a partial stream never yields rows.
fn read_measure_batch_stream(
    stream: &mut TcpStream,
    expected_inputs: usize,
    request_bytes: usize,
) -> CliResult<MeasureBatchAtResponse> {
    let mut response_bytes = 0usize;
    let mut next_frame = |stream: &mut TcpStream| -> CliResult<ResidentMeasureBatchStreamFrame> {
        let frame = read_frame(stream)?;
        response_bytes += frame.len();
        Ok(decode_binary::<ResidentMeasureBatchStreamFrame>(&frame)?)
    };
    let header = match next_frame(stream)? {
        ResidentMeasureBatchStreamFrame::Header(header) => header,
        ResidentMeasureBatchStreamFrame::Err {
            code,
            message,
            remediation,
        } => return Err(remote_stream_error(&code, &message, &remediation)),
        other => return Err(unexpected_stream_frame("Header", &other)),
    };
    validate_measure_batch_header(&header)?;
    let mut rows: Vec<ResidentMeasuredInput> = Vec::with_capacity(header.input_count);
    let end = loop {
        match next_frame(stream)? {
            ResidentMeasureBatchStreamFrame::Row(row) => {
                if row.input_index != rows.len() {
                    return Err(CliError::from(CalyxError {
                        code: "CALYX_PANEL_RESIDENT_STREAM_ORDER",
                        message: format!(
                            "resident measure_batch row frame carries input_index {} but {} rows were received",
                            row.input_index,
                            rows.len()
                        ),
                        remediation: "restart the resident service from the same Calyx build as the CLI",
                    }));
                }
                rows.push(*row);
            }
            ResidentMeasureBatchStreamFrame::End(end) => break end,
            ResidentMeasureBatchStreamFrame::Err {
                code,
                message,
                remediation,
            } => return Err(remote_stream_error(&code, &message, &remediation)),
            other => return Err(unexpected_stream_frame("Row or End", &other)),
        }
    };
    if end.row_count != rows.len() || rows.len() != expected_inputs {
        return Err(CliError::from(CalyxError {
            code: "CALYX_PANEL_RESIDENT_STREAM_ORDER",
            message: format!(
                "resident measure_batch stream ended with {} rows (end frame says {}) for {} inputs",
                rows.len(),
                end.row_count,
                expected_inputs
            ),
            remediation: "restart the resident service from the same Calyx build as the CLI",
        }));
    }
    Ok(MeasureBatchAtResponse {
        response: MeasureBatchResponse {
            schema: header.schema,
            ready: header.ready,
            process_id: header.process_id,
            template_source: header.template_source,
            modality: header.modality,
            input_count: header.input_count,
            elapsed_ms: end.elapsed_ms,
            runtime_batch_limit: header.runtime_batch_limit,
            rows,
        },
        request_bytes,
        response_bytes,
    })
}

fn read_measure_batch_summary_stream(
    stream: &mut TcpStream,
    expected_inputs: usize,
    request_bytes: usize,
) -> CliResult<MeasureBatchSummaryResponse> {
    let mut response_bytes = 0usize;
    let frame = read_frame(stream)?;
    response_bytes += frame.len();
    let header = match decode_binary::<ResidentMeasureBatchStreamFrame>(&frame)? {
        ResidentMeasureBatchStreamFrame::Header(header) => header,
        ResidentMeasureBatchStreamFrame::Err {
            code,
            message,
            remediation,
        } => return Err(remote_stream_error(&code, &message, &remediation)),
        other => return Err(unexpected_stream_frame("Header", &other)),
    };
    validate_measure_batch_header(&header)?;
    let mut hasher = Sha256::new();
    let mut row_count = 0usize;
    let mut measured_slot_counts = Vec::new();
    let mut absent_slot_counts = Vec::new();
    let end = loop {
        let frame = read_frame(stream)?;
        response_bytes += frame.len();
        match decode_binary::<ResidentMeasureBatchStreamFrame>(&frame)? {
            ResidentMeasureBatchStreamFrame::Row(row) => {
                if row.input_index != row_count {
                    return Err(CliError::from(CalyxError {
                        code: "CALYX_PANEL_RESIDENT_STREAM_ORDER",
                        message: format!(
                            "resident measure_batch row frame carries input_index {} but {} rows were received",
                            row.input_index, row_count
                        ),
                        remediation: "restart the resident service from the same Calyx build as the CLI",
                    }));
                }
                hasher.update(&frame);
                push_unique(&mut measured_slot_counts, row.measured_slot_count);
                push_unique(&mut absent_slot_counts, row.absent_slot_count);
                row_count += 1;
            }
            ResidentMeasureBatchStreamFrame::End(end) => break end,
            ResidentMeasureBatchStreamFrame::Err {
                code,
                message,
                remediation,
            } => return Err(remote_stream_error(&code, &message, &remediation)),
            other => return Err(unexpected_stream_frame("Row or End", &other)),
        }
    };
    if end.row_count != row_count || row_count != expected_inputs {
        return Err(CliError::from(CalyxError {
            code: "CALYX_PANEL_RESIDENT_STREAM_ORDER",
            message: format!(
                "resident measure_batch stream ended with {row_count} rows (end frame says {}) for {expected_inputs} inputs",
                end.row_count
            ),
            remediation: "restart the resident service from the same Calyx build as the CLI",
        }));
    }
    Ok(MeasureBatchSummaryResponse {
        schema: header.schema,
        ready: header.ready,
        process_id: header.process_id,
        template_source: header.template_source,
        modality: header.modality,
        input_count: header.input_count,
        elapsed_ms: end.elapsed_ms,
        runtime_batch_limit: header.runtime_batch_limit,
        row_count,
        measured_slot_counts,
        absent_slot_counts,
        response_rows_sha256: hex_digest(&hasher.finalize()),
        request_bytes,
        response_bytes,
    })
}

fn validate_measure_batch_header(header: &ResidentMeasureBatchStreamHeader) -> CliResult {
    if header.protocol_version != RESIDENT_BINARY_PROTOCOL_VERSION {
        return Err(CliError::from(CalyxError {
            code: "CALYX_PANEL_RESIDENT_PROTOCOL_MISMATCH",
            message: format!(
                "resident measure_batch binary protocol {}, expected {}",
                header.protocol_version, RESIDENT_BINARY_PROTOCOL_VERSION
            ),
            remediation: "restart the resident service from the same Calyx build as the CLI",
        }));
    }
    if header.schema != MEASURE_BATCH_SCHEMA {
        return Err(CliError::from(CalyxError {
            code: "CALYX_PANEL_RESIDENT_SCHEMA_MISMATCH",
            message: format!(
                "resident measure_batch schema {}, expected {}",
                header.schema, MEASURE_BATCH_SCHEMA
            ),
            remediation: "restart the resident service from the same Calyx build as the CLI",
        }));
    }
    Ok(())
}

fn push_unique(values: &mut Vec<usize>, value: usize) {
    if !values.contains(&value) {
        values.push(value);
        values.sort_unstable();
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub(super) fn remote_stream_error(code: &str, message: &str, remediation: &str) -> CliError {
    if let Some((declared_code, declared_remediation)) = declared_stream_error(code, remediation) {
        return CliError::from(CalyxError {
            code: declared_code,
            message: message.to_string(),
            remediation: declared_remediation,
        });
    }
    CliError::from(CalyxError {
        code: "CALYX_PANEL_RESIDENT_PROTOCOL_MISMATCH",
        message: format!(
            "resident returned an undeclared error envelope: code={code:?}, message={message:?}, remediation={remediation:?}"
        ),
        remediation: "restart the resident service from the same Calyx build as the CLI",
    })
}

/// Resolve only error identities declared by this build. A resident error is
/// a protocol value, not arbitrary data: accepting an unknown code or a known
/// code with different remediation would silently corrupt the CLI's closed
/// error catalog. Core catalog entries are already authoritative; resident
/// subsystem pairs are enumerated here and pinned by tests.
fn declared_stream_error(code: &str, remediation: &str) -> Option<(&'static str, &'static str)> {
    if let Some(entry) = calyx_core::CALYX_ERROR_CODES
        .iter()
        .find(|entry| entry.code() == code && entry.remediation() == remediation)
    {
        return Some((entry.code(), entry.remediation()));
    }
    if let Some(error) = super::input_wire::declared_error(code, remediation) {
        return Some(error);
    }
    const RESTART: &str = "restart the resident service from the same Calyx build as the CLI";
    const REDUCE_FRAME: &str =
        "reduce the measurement batch size or implement streaming vector payloads";
    const START_RESIDENT: &str =
        "start `calyx panel resident serve` on the requested loopback address";
    const LOCAL: &[(&str, &str)] = &[
        ("CALYX_PANEL_RESIDENT_BINARY_DECODE", RESTART),
        ("CALYX_PANEL_RESIDENT_BINARY_FRAME", RESTART),
        ("CALYX_PANEL_RESIDENT_BINARY_FRAME", REDUCE_FRAME),
        ("CALYX_PANEL_RESIDENT_BINARY_FRAME", START_RESIDENT),
        ("CALYX_PANEL_RESIDENT_PROTOCOL_MISMATCH", RESTART),
        (
            calyx_search::SEARCH_PIN_BUDGET_EXCEEDED_CODE,
            calyx_search::SEARCH_PIN_BUDGET_EXCEEDED_REMEDIATION,
        ),
        (
            super::stream::EMPTY_INPUT,
            super::stream::EMPTY_INPUT_REMEDIATION,
        ),
        (
            super::stream::RUNTIME_BATCH_LIMIT_EXCEEDED,
            super::stream::RUNTIME_BATCH_LIMIT_REMEDIATION,
        ),
        (
            "CALYX_PANEL_RESIDENT_MEASURE_WINDOW_INVALID",
            "set CALYX_PANEL_RESIDENT_MEASURE_WINDOW to a positive integer or unset it",
        ),
        (
            "CALYX_EMBED_REQUIRE_PARALLEL_INVALID",
            "set CALYX_EMBED_REQUIRE_PARALLEL to error (default), warn, or off",
        ),
        (
            "CALYX_EMBED_OVERLAP_FLOOR_INVALID",
            "set CALYX_EMBED_OVERLAP_FLOOR_MS to a non-negative integer (default 25), or unset it",
        ),
        (
            "CALYX_EMBED_SEQUENTIAL_EXECUTION",
            "embedders must run concurrently: find what serialized the slots (shared lock, serial loop regression); set CALYX_EMBED_REQUIRE_PARALLEL=warn only to diagnose",
        ),
        (
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            "send op=ready, measure, measure_batch, or shutdown",
        ),
        (
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            "send a modality such as text, code, image, audio, protein, or dna",
        ),
        (
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            "send UTF-8 text as input or arbitrary bytes as lowercase input_hex",
        ),
        (
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            "send inputs_hex as an array of even-length hexadecimal byte strings",
        ),
        (
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            "send one JSON object per connection with op=ready, measure, or shutdown",
        ),
        (
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            "send one JSON object per connection or the resident binary magic line",
        ),
        (
            "CALYX_PANEL_RESIDENT_INPUT_HEX_INVALID",
            "send an even-length hexadecimal input_hex string",
        ),
        (
            "CALYX_PANEL_RESIDENT_INPUT_HEX_INVALID",
            "send each inputs_hex item as an even-length hexadecimal byte string",
        ),
    ];
    LOCAL
        .iter()
        .copied()
        .find(|candidate| candidate.0 == code && candidate.1 == remediation)
}

fn unexpected_stream_frame(expected: &str, got: &ResidentMeasureBatchStreamFrame) -> CliError {
    let kind = match got {
        ResidentMeasureBatchStreamFrame::Header(_) => "Header",
        ResidentMeasureBatchStreamFrame::Row(_) => "Row",
        ResidentMeasureBatchStreamFrame::End(_) => "End",
        ResidentMeasureBatchStreamFrame::Err { .. } => "Err",
    };
    CliError::from(CalyxError {
        code: "CALYX_PANEL_RESIDENT_STREAM_ORDER",
        message: format!(
            "resident measure_batch stream sent a {kind} frame where {expected} was expected"
        ),
        remediation: "restart the resident service from the same Calyx build as the CLI",
    })
}
