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

fn send_binary_measure_batch_request(
    addr: SocketAddr,
    request: &ResidentMeasureBatchBinaryRequest,
) -> CliResult<(ResidentMeasureBatchBinaryResponse, usize, usize)> {
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
    let request_bytes = encode_binary(request)?;
    write_frame(&mut stream, &request_bytes)?;
    stream.flush()?;
    let response_frame = read_frame(&mut stream)?;
    let response_bytes = response_frame.len();
    let response = decode_binary::<ResidentMeasureBatchBinaryResponse>(&response_frame)?;
    Ok((response, request_bytes.len(), response_bytes))
}

pub(crate) fn measure_batch_at(
    addr: SocketAddr,
    modality: Modality,
    inputs: &[Input],
    runtime_batch_limit: Option<usize>,
) -> CliResult<MeasureBatchAtResponse> {
    let request = ResidentMeasureBatchBinaryRequest {
        protocol_version: RESIDENT_BINARY_PROTOCOL_VERSION,
        modality,
        inputs: inputs
            .iter()
            .map(|input| input.bytes.clone())
            .collect::<Vec<_>>(),
        runtime_batch_limit,
    };
    let (response, request_bytes, response_bytes) =
        send_binary_measure_batch_request(addr, &request)?;
    if response.protocol_version != RESIDENT_BINARY_PROTOCOL_VERSION {
        return Err(CliError::from(CalyxError {
            code: "CALYX_PANEL_RESIDENT_PROTOCOL_MISMATCH",
            message: format!(
                "resident measure_batch binary protocol {}, expected {}",
                response.protocol_version, RESIDENT_BINARY_PROTOCOL_VERSION
            ),
            remediation: "restart the resident service from the same Calyx build as the CLI",
        }));
    }
    let parsed = match response.result {
        ResidentMeasureBatchBinaryResult::Ok(response) => response,
        ResidentMeasureBatchBinaryResult::Err {
            code,
            message,
            remediation,
        } => {
            return Err(CliError::from(CalyxError {
                code: resident_remote_error_code(&code),
                message: format!("{code}: {message}; remediation={remediation}"),
                remediation: CLIENT_TIMEOUT_REMEDIATION,
            }));
        }
    };
    if parsed.schema != MEASURE_BATCH_SCHEMA {
        return Err(CliError::from(CalyxError {
            code: "CALYX_PANEL_RESIDENT_SCHEMA_MISMATCH",
            message: format!(
                "resident measure_batch schema {}, expected {}",
                parsed.schema, MEASURE_BATCH_SCHEMA
            ),
            remediation: "restart the resident service from the same Calyx build as the CLI",
        }));
    }
    Ok(MeasureBatchAtResponse {
        response: parsed,
        request_bytes,
        response_bytes,
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
