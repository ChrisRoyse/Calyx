use super::*;

pub(crate) fn encode_binary(value: &impl Serialize) -> Result<Vec<u8>, CalyxError> {
    bincode::serde::encode_to_vec(value, config::standard()).map_err(|error| CalyxError {
        code: "CALYX_PANEL_RESIDENT_BINARY_ENCODE",
        message: format!("encode resident binary frame failed: {error}"),
        remediation: "restart the resident service from the same Calyx build as the CLI",
    })
}

pub(crate) fn decode_binary<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, CalyxError> {
    let (value, consumed) =
        bincode::serde::decode_from_slice(bytes, config::standard()).map_err(|error| {
            CalyxError {
                code: "CALYX_PANEL_RESIDENT_BINARY_DECODE",
                message: format!("decode resident binary frame failed: {error}"),
                remediation: "restart the resident service from the same Calyx build as the CLI",
            }
        })?;
    if consumed != bytes.len() {
        return Err(CalyxError {
            code: "CALYX_PANEL_RESIDENT_BINARY_DECODE",
            message: format!(
                "decode resident binary frame consumed {consumed} of {} bytes",
                bytes.len()
            ),
            remediation: "restart the resident service from the same Calyx build as the CLI",
        });
    }
    Ok(value)
}

pub(crate) fn write_binary_response(
    stream: &mut TcpStream,
    response: &ResidentMeasureBatchBinaryResponse,
) -> CliResult {
    let bytes = encode_binary(response)?;
    eprintln!(
        "CALYX_PANEL_RESIDENT_RUNTIME phase=measure_batch_binary_response process_id={} protocol_version={} response_bytes={}",
        std::process::id(),
        response.protocol_version,
        bytes.len()
    );
    write_frame(stream, &bytes)?;
    Ok(())
}

pub(crate) fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> Result<(), CalyxError> {
    if bytes.len() > MAX_RESIDENT_SERVICE_FRAME_BYTES {
        return Err(CalyxError {
            code: "CALYX_PANEL_RESIDENT_BINARY_FRAME",
            message: format!(
                "resident service binary frame {} bytes exceeds max {}",
                bytes.len(),
                MAX_RESIDENT_SERVICE_FRAME_BYTES
            ),
            remediation: "reduce the measurement batch size or implement streaming vector payloads",
        });
    }
    let len = u64::try_from(bytes.len()).map_err(|_| CalyxError {
        code: "CALYX_PANEL_RESIDENT_BINARY_FRAME",
        message: format!(
            "resident service binary frame {} bytes overflows u64",
            bytes.len()
        ),
        remediation: "reduce the measurement batch size or implement streaming vector payloads",
    })?;
    writer
        .write_all(&len.to_be_bytes())
        .and_then(|_| writer.write_all(bytes))
        .map_err(|error| CalyxError {
            code: "CALYX_PANEL_RESIDENT_BINARY_FRAME",
            message: format!("write resident service binary frame failed: {error}"),
            remediation: CLIENT_TIMEOUT_REMEDIATION,
        })
}

pub(crate) fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>, CalyxError> {
    let mut header = [0_u8; 8];
    let mut offset = 0;
    while offset < header.len() {
        match reader.read(&mut header[offset..]) {
            Ok(0) => {
                return Err(CalyxError {
                    code: "CALYX_PANEL_RESIDENT_BINARY_FRAME",
                    message: format!(
                        "read resident service binary frame header failed: stream closed after {offset} of 8 bytes"
                    ),
                    remediation: CLIENT_TIMEOUT_REMEDIATION,
                });
            }
            Ok(n) => offset += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(CalyxError {
                    code: "CALYX_PANEL_RESIDENT_BINARY_FRAME",
                    message: format!("read resident service binary frame header failed: {error}"),
                    remediation: CLIENT_TIMEOUT_REMEDIATION,
                });
            }
        }
    }
    let len = u64::from_be_bytes(header);
    let len = usize::try_from(len).map_err(|_| CalyxError {
        code: "CALYX_PANEL_RESIDENT_BINARY_FRAME",
        message: format!("resident service binary frame length {len} overflows usize"),
        remediation: "reduce the measurement batch size or implement streaming vector payloads",
    })?;
    if len > MAX_RESIDENT_SERVICE_FRAME_BYTES {
        return Err(CalyxError {
            code: "CALYX_PANEL_RESIDENT_BINARY_FRAME",
            message: format!(
                "resident service binary frame {len} bytes exceeds max {MAX_RESIDENT_SERVICE_FRAME_BYTES}"
            ),
            remediation: "reduce the measurement batch size or implement streaming vector payloads",
        });
    }
    let mut body = vec![0_u8; len];
    reader.read_exact(&mut body).map_err(|error| CalyxError {
        code: "CALYX_PANEL_RESIDENT_BINARY_FRAME",
        message: format!("read resident service binary frame body ({len} bytes) failed: {error}"),
        remediation: CLIENT_TIMEOUT_REMEDIATION,
    })?;
    Ok(body)
}
