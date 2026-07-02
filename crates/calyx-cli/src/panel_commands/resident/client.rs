use super::codec::{decode_binary, encode_binary, read_frame, write_frame};
use super::*;

pub(crate) fn client_command(args: &[String], op: &str) -> CliResult {
    let flags = parse_client_flags(args, op)?;
    let mut request = json!({ "op": op });
    if op == "measure" {
        request["modality"] = serde_json::to_value(flags.modality.expect("parsed modality"))?;
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

/// Programmatic readiness probe used by ingest resident-route discovery: one
/// JSON `ready` round-trip returning the raw readiness value.
pub(crate) fn ready_value_at(addr: SocketAddr) -> CliResult<Value> {
    send_request(addr, json!({ "op": "ready" }))
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
    let timeout = Some(Duration::from_secs(CLIENT_TIMEOUT_SECS));
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    Ok(serde_json::from_str(&response)?)
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
    let timeout = Some(Duration::from_secs(CLIENT_TIMEOUT_SECS));
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    stream.write_all(RESIDENT_BINARY_MAGIC)?;
    let request_bytes = encode_binary(&ResidentMeasureBatchBinaryRequest {
        protocol_version: RESIDENT_BINARY_PROTOCOL_VERSION,
        modality,
        inputs: inputs
            .iter()
            .map(|input| input.bytes.clone())
            .collect::<Vec<_>>(),
        runtime_batch_limit,
    })?;
    write_frame(&mut stream, &request_bytes)?;
    stream.flush()?;
    read_measure_batch_stream(&mut stream, inputs.len(), request_bytes.len())
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

fn remote_stream_error(code: &str, message: &str, remediation: &str) -> CliError {
    CliError::from(CalyxError {
        code: resident_remote_error_code(code),
        message: format!("{code}: {message}; remediation={remediation}"),
        remediation: CLIENT_TIMEOUT_REMEDIATION,
    })
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

fn resident_remote_error_code(remote_code: &str) -> &'static str {
    match remote_code {
        "CALYX_PANEL_RESIDENT_BAD_REQUEST" => "CALYX_PANEL_RESIDENT_BAD_REQUEST",
        "CALYX_PANEL_RESIDENT_INPUT_HEX_INVALID" => "CALYX_PANEL_RESIDENT_INPUT_HEX_INVALID",
        "CALYX_PANEL_RESIDENT_UNAVAILABLE" => "CALYX_PANEL_RESIDENT_UNAVAILABLE",
        "CALYX_PANEL_RESIDENT_SCHEMA_MISMATCH" => "CALYX_PANEL_RESIDENT_SCHEMA_MISMATCH",
        "CALYX_PANEL_RESIDENT_BINARY_ENCODE" => "CALYX_PANEL_RESIDENT_BINARY_ENCODE",
        "CALYX_PANEL_RESIDENT_BINARY_DECODE" => "CALYX_PANEL_RESIDENT_BINARY_DECODE",
        "CALYX_PANEL_RESIDENT_BINARY_FRAME" => "CALYX_PANEL_RESIDENT_BINARY_FRAME",
        "CALYX_PANEL_RESIDENT_PROTOCOL_MISMATCH" => "CALYX_PANEL_RESIDENT_PROTOCOL_MISMATCH",
        _ => "CALYX_PANEL_RESIDENT_ERROR",
    }
}
