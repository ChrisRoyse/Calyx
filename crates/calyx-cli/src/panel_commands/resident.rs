use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use bincode::config;
use calyx_core::{AbsentReason, CalyxError, Input, LensId, Modality, SlotState, SlotVector};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

mod flags;
mod protocol;

use flags::{
    ServeFlags, calyx_home, ensure_loopback, parse_addr, parse_client_flags, parse_serve_flags,
};
use protocol::{
    ClientMeasureInput, MEASURE_BATCH_SCHEMA, MEASURE_SCHEMA, MeasureResponse, READY_SCHEMA,
    RESIDENT_BINARY_PROTOCOL_VERSION, ReadyResponse, ResidentMeasureBatchBinaryRequest,
    ResidentMeasureBatchBinaryResponse, ResidentMeasureBatchBinaryResult, ResidentRequest,
    hex_decode,
};
pub(crate) use protocol::{
    MeasureBatchAtResponse, MeasureBatchResponse, ResidentMeasuredInput, ResidentSlotMeasure,
};

use super::warm::resident_support::{
    ResidentWarmOptions, ResidentWarmState, load_resident_warm_state,
};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DEFAULT_BIND: &str = "127.0.0.1:8787";
const DEFAULT_MAX_RESIDENT_VRAM_MIB: u64 = 22 * 1024;
const DEFAULT_RESIDENT_OVERHEAD_MULTIPLIER_MILLI: u64 = 2100;
const DEFAULT_MAX_LOAD_SECS: u64 = 60;
const CLIENT_TIMEOUT_SECS: u64 = 30;
const CLIENT_TIMEOUT_REMEDIATION: &str =
    "start `calyx panel resident serve` on the requested loopback address";
const RESIDENT_BINARY_MAGIC: &[u8] = b"CALYX_PANEL_RESIDENT_BIN1\n";
const MAX_RESIDENT_SERVICE_FRAME_BYTES: usize = 2 * 1024 * 1024 * 1024;

struct ResidentService {
    state: ResidentWarmState,
    bind: SocketAddr,
    started: Instant,
}

pub(crate) fn run(args: &[String]) -> CliResult {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(CliError::usage(
            "calyx panel resident requires serve, ready, measure, or stop",
        ));
    };
    match command {
        "serve" => serve(&args[1..]),
        "ready" => client_command(&args[1..], "ready"),
        "measure" => client_command(&args[1..], "measure"),
        "stop" => client_command(&args[1..], "shutdown"),
        other => Err(CliError::usage(format!(
            "unknown panel resident subcommand {other}; expected serve, ready, measure, or stop"
        ))),
    }
}

fn serve(args: &[String]) -> CliResult {
    let mut flags = parse_serve_flags(args)?;
    let bind = flags.bind.unwrap_or(parse_addr(DEFAULT_BIND)?);
    ensure_loopback(bind)?;
    let home = resolve_home(&mut flags)?;
    let template = flags.template.take().ok_or_else(|| {
        CliError::usage("calyx panel resident serve requires --template <name-or-id>")
    })?;
    let listener = TcpListener::bind(bind)?;
    let local_addr = listener.local_addr()?;
    let state = load_resident_warm_state(warm_options(home, template, flags))?;
    let service = Arc::new(ResidentService {
        state,
        bind: local_addr,
        started: Instant::now(),
    });
    let ready = readiness(&service);
    if let Some(path) = service.state.ready_out.clone() {
        write_json_file(path, &ready)?;
    }
    print_json(&ready)?;
    serve_loop(listener, service)
}

fn resolve_home(flags: &mut ServeFlags) -> CliResult<PathBuf> {
    resolve_home_with(flags.home.take(), calyx_home)
}

fn resolve_home_with(
    provided: Option<PathBuf>,
    fallback: impl FnOnce() -> CliResult<PathBuf>,
) -> CliResult<PathBuf> {
    match provided {
        Some(home) => Ok(home),
        None => fallback(),
    }
}

fn warm_options(home: PathBuf, template: String, flags: ServeFlags) -> ResidentWarmOptions {
    ResidentWarmOptions {
        home,
        template,
        ready_out: flags.ready_out,
        max_resident_vram_mib: flags
            .max_resident_vram_mib
            .unwrap_or(DEFAULT_MAX_RESIDENT_VRAM_MIB),
        resident_overhead_multiplier_milli: flags
            .resident_overhead_multiplier_milli
            .unwrap_or(DEFAULT_RESIDENT_OVERHEAD_MULTIPLIER_MILLI),
        max_load_secs: flags.max_load_secs.unwrap_or(DEFAULT_MAX_LOAD_SECS),
        load_parallelism: flags.load_parallelism,
        progress_out: flags.progress_out,
    }
}

fn serve_loop(listener: TcpListener, service: Arc<ResidentService>) -> CliResult {
    let running = Arc::new(AtomicBool::new(true));
    while running.load(Ordering::SeqCst) {
        let (stream, peer) = listener.accept()?;
        if !peer.ip().is_loopback() {
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        }
        handle_client(stream, Arc::clone(&service), Arc::clone(&running))?;
    }
    Ok(())
}

fn client_command(args: &[String], op: &str) -> CliResult {
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

fn handle_client(
    mut stream: TcpStream,
    service: Arc<ResidentService>,
    running: Arc<AtomicBool>,
) -> CliResult {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut first_line = Vec::new();
    reader.read_until(b'\n', &mut first_line)?;
    if first_line == RESIDENT_BINARY_MAGIC {
        let response = dispatch_binary_measure_batch(&mut reader, &service);
        write_binary_response(&mut stream, &response)?;
        stream.flush()?;
        let _ = stream.shutdown(Shutdown::Both);
        return Ok(());
    }

    let response = match String::from_utf8(first_line) {
        Ok(line) => match serde_json::from_str::<ResidentRequest>(&line) {
            Ok(request) => dispatch_request(request, &service, &running),
            Err(error) => error_value(
                "CALYX_PANEL_RESIDENT_BAD_REQUEST",
                format!("decode resident request JSON line: {error}"),
                "send one JSON object per connection with op=ready, measure, or shutdown",
            ),
        },
        Err(error) => error_value(
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            format!("resident request was neither binary magic nor valid UTF-8 JSON: {error}"),
            "send one JSON object per connection or the resident binary magic line",
        ),
    };
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}

fn dispatch_binary_measure_batch(
    reader: &mut impl Read,
    service: &ResidentService,
) -> ResidentMeasureBatchBinaryResponse {
    let result = match read_frame(reader).and_then(|bytes| {
        decode_binary::<ResidentMeasureBatchBinaryRequest>(&bytes)
            .map(|request| (bytes.len(), request))
    }) {
        Ok((request_bytes, request)) => {
            if request.protocol_version != RESIDENT_BINARY_PROTOCOL_VERSION {
                ResidentMeasureBatchBinaryResult::Err {
                    code: "CALYX_PANEL_RESIDENT_PROTOCOL_MISMATCH".to_string(),
                    message: format!(
                        "resident binary measure_batch protocol version {}, expected {}",
                        request.protocol_version, RESIDENT_BINARY_PROTOCOL_VERSION
                    ),
                    remediation:
                        "restart the resident service from the same Calyx build as the CLI"
                            .to_string(),
                }
            } else {
                eprintln!(
                    "CALYX_PANEL_RESIDENT_RUNTIME phase=measure_batch_binary_request process_id={} protocol_version={} request_bytes={} inputs={}",
                    std::process::id(),
                    request.protocol_version,
                    request_bytes,
                    request.inputs.len()
                );
                match measure_batch(
                    service,
                    request.modality,
                    request.inputs,
                    request.runtime_batch_limit,
                ) {
                    Ok(response) => ResidentMeasureBatchBinaryResult::Ok(response),
                    Err(error) => ResidentMeasureBatchBinaryResult::Err {
                        code: error.code().to_string(),
                        message: error.message().to_string(),
                        remediation: error.remediation().to_string(),
                    },
                }
            }
        }
        Err(error) => ResidentMeasureBatchBinaryResult::Err {
            code: error.code.to_string(),
            message: error.message,
            remediation: error.remediation.to_string(),
        },
    };
    ResidentMeasureBatchBinaryResponse {
        protocol_version: RESIDENT_BINARY_PROTOCOL_VERSION,
        result,
    }
}

fn dispatch_request(
    request: ResidentRequest,
    service: &ResidentService,
    running: &AtomicBool,
) -> Value {
    match request.op.as_str() {
        "ready" => json!(readiness(service)),
        "measure" => dispatch_measure(request, service),
        "measure_batch" => dispatch_measure_batch(request, service),
        "shutdown" => {
            running.store(false, Ordering::SeqCst);
            json!({"ok": true, "schema": READY_SCHEMA, "ready": false, "stopping": true})
        }
        other => error_value(
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            format!("unknown resident op {other}"),
            "send op=ready, measure, measure_batch, or shutdown",
        ),
    }
}

fn dispatch_measure(request: ResidentRequest, service: &ResidentService) -> Value {
    let Some(modality) = request.modality else {
        return error_value(
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            "measure requires modality",
            "send a modality such as text, code, image, audio, protein, or dna",
        );
    };
    let bytes = match request_input_bytes(request.input, request.input_hex) {
        Ok(bytes) => bytes,
        Err(error) => return error,
    };
    match measure(service, modality, bytes) {
        Ok(response) => json!(response),
        Err(error) => cli_error_value(&error),
    }
}

fn dispatch_measure_batch(request: ResidentRequest, service: &ResidentService) -> Value {
    let Some(modality) = request.modality else {
        return error_value(
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            "measure_batch requires modality",
            "send a modality such as text, code, image, audio, protein, or dna",
        );
    };
    let bytes = match request_inputs_bytes(request.inputs_hex) {
        Ok(bytes) => bytes,
        Err(error) => return error,
    };
    match measure_batch(service, modality, bytes, request.runtime_batch_limit) {
        Ok(response) => json!(response),
        Err(error) => cli_error_value(&error),
    }
}

fn request_input_bytes(input: Option<String>, input_hex: Option<String>) -> Result<Vec<u8>, Value> {
    match (input, input_hex) {
        (Some(_), Some(_)) => Err(error_value(
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            "measure accepts exactly one of input or input_hex",
            "send UTF-8 text as input or arbitrary bytes as lowercase input_hex",
        )),
        (Some(text), None) => Ok(text.into_bytes()),
        (None, Some(hex)) => hex_decode(&hex).map_err(|message| {
            error_value(
                "CALYX_PANEL_RESIDENT_INPUT_HEX_INVALID",
                message,
                "send an even-length hexadecimal input_hex string",
            )
        }),
        (None, None) => Err(error_value(
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            "measure requires input or input_hex",
            "send UTF-8 text as input or arbitrary bytes as lowercase input_hex",
        )),
    }
}

fn request_inputs_bytes(inputs_hex: Option<Vec<String>>) -> Result<Vec<Vec<u8>>, Value> {
    let Some(inputs_hex) = inputs_hex else {
        return Err(error_value(
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            "measure_batch requires inputs_hex",
            "send inputs_hex as an array of even-length hexadecimal byte strings",
        ));
    };
    inputs_hex
        .into_iter()
        .enumerate()
        .map(|(index, hex)| {
            hex_decode(&hex).map_err(|message| {
                error_value(
                    "CALYX_PANEL_RESIDENT_INPUT_HEX_INVALID",
                    format!("inputs_hex[{index}]: {message}"),
                    "send each inputs_hex item as an even-length hexadecimal byte string",
                )
            })
        })
        .collect()
}

fn readiness(service: &ResidentService) -> ReadyResponse {
    let state = &service.state;
    ReadyResponse {
        schema: READY_SCHEMA.to_string(),
        ready: true,
        residency_scope: "resident_service_process",
        process_id: std::process::id(),
        bind: service.bind,
        uptime_ms: service.started.elapsed().as_millis(),
        source_of_truth: state.source_of_truth.clone(),
        home: state.home.clone(),
        template_selector: state.template_selector.clone(),
        template_source: state.template_source.clone(),
        ready_out: state.ready_out.clone(),
        max_resident_vram_mib: state.max_resident_vram_mib,
        declared_template_vram_mib: state.declared_template_vram_mib,
        resident_overhead_multiplier: state.resident_overhead_multiplier,
        estimated_resident_vram_mib: state.estimated_resident_vram_mib,
        max_load_secs: state.max_load_secs,
        load_parallelism: state.load_parallelism,
        load_ms: state.load_ms,
        probe_ms: state.probe_ms,
        slot_count: state.build.panel.slots.len(),
        content_lens_count: state.content_lens_count,
        registry_lens_count: state.build.registry.lens_snapshots().len(),
        warmed_lens_count: state.warmed_lens_count,
        gpu_content_lens_count: state.gpu_content_lens_count,
        cpu_content_lens_count: state
            .content_lens_count
            .saturating_sub(state.gpu_content_lens_count),
    }
}

fn measure(
    service: &ResidentService,
    modality: Modality,
    bytes: Vec<u8>,
) -> CliResult<MeasureResponse> {
    let started = Instant::now();
    let input = Input::new(modality, bytes);
    let mut measured = 0;
    let mut absent = 0;
    let mut slots = Vec::new();
    for slot in &service.state.build.panel.slots {
        let (measured_slot, vector, absent_reason) = if slot.state != SlotState::Active {
            (false, None, Some(AbsentReason::LensInactive))
        } else if slot.modality != modality {
            (false, None, Some(AbsentReason::NotApplicable))
        } else if !service.state.build.registry.contains(slot.lens_id) {
            (false, None, Some(AbsentReason::LensUnavailable))
        } else {
            let vector = service.state.build.registry.measure(slot.lens_id, &input)?;
            (true, Some(vector), None)
        };
        if measured_slot {
            measured += 1;
        } else {
            absent += 1;
        }
        slots.push(slot_measure(slot, measured_slot, vector, absent_reason));
    }
    Ok(MeasureResponse {
        schema: MEASURE_SCHEMA.to_string(),
        ready: true,
        process_id: std::process::id(),
        template_source: service.state.template_source.clone(),
        modality,
        input_len: input.bytes.len(),
        elapsed_ms: started.elapsed().as_millis(),
        measured_slot_count: measured,
        absent_slot_count: absent,
        slots,
    })
}

fn measure_batch(
    service: &ResidentService,
    modality: Modality,
    input_bytes: Vec<Vec<u8>>,
    runtime_batch_limit: Option<usize>,
) -> CliResult<MeasureBatchResponse> {
    if matches!(runtime_batch_limit, Some(0)) {
        return Err(CalyxError::lens_unreachable(
            "resident measure_batch runtime_batch_limit must be > 0 when supplied",
        )
        .into());
    }
    let started = Instant::now();
    eprintln!(
        "CALYX_PANEL_RESIDENT_RUNTIME phase=measure_batch_start process_id={} modality={:?} inputs={} runtime_batch_limit={:?}",
        std::process::id(),
        modality,
        input_bytes.len(),
        runtime_batch_limit
    );
    let inputs = input_bytes
        .into_iter()
        .map(|bytes| Input::new(modality, bytes))
        .collect::<Vec<_>>();
    let mut measured_by_lens = BTreeMap::<LensId, Vec<SlotVector>>::new();
    for slot in &service.state.build.panel.slots {
        if slot.state != SlotState::Active
            || slot.modality != modality
            || !service.state.build.registry.contains(slot.lens_id)
            || measured_by_lens.contains_key(&slot.lens_id)
        {
            continue;
        }
        let lens_started = Instant::now();
        let vectors = measure_lens_batch_with_limit(
            &service.state.build.registry,
            slot.lens_id,
            &inputs,
            runtime_batch_limit,
        )?;
        if vectors.len() != inputs.len() {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "resident measure_batch lens {} returned {} vectors for {} inputs",
                slot.lens_id,
                vectors.len(),
                inputs.len()
            ))
            .into());
        }
        eprintln!(
            "CALYX_PANEL_RESIDENT_RUNTIME phase=measure_batch_lens_ok process_id={} lens_id={} slot={} inputs={} elapsed_ms={}",
            std::process::id(),
            slot.lens_id,
            slot.slot_id.get(),
            inputs.len(),
            lens_started.elapsed().as_millis()
        );
        measured_by_lens.insert(slot.lens_id, vectors);
    }
    let mut rows = Vec::with_capacity(inputs.len());
    for (input_index, input) in inputs.iter().enumerate() {
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
                    .and_then(|vectors| vectors.get(input_index))
                    .cloned()
                    .ok_or_else(|| {
                        CalyxError::lens_unreachable(format!(
                            "resident measure_batch missing measured vector for lens {} input {}",
                            slot.lens_id, input_index
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
        rows.push(ResidentMeasuredInput {
            input_index,
            input_len: input.bytes.len(),
            measured_slot_count: measured,
            absent_slot_count: absent,
            slots,
        });
    }
    let elapsed_ms = started.elapsed().as_millis();
    eprintln!(
        "CALYX_PANEL_RESIDENT_RUNTIME phase=measure_batch_ok process_id={} modality={:?} inputs={} elapsed_ms={}",
        std::process::id(),
        modality,
        rows.len(),
        elapsed_ms
    );
    Ok(MeasureBatchResponse {
        schema: MEASURE_BATCH_SCHEMA.to_string(),
        ready: true,
        process_id: std::process::id(),
        template_source: service.state.template_source.clone(),
        modality,
        input_count: inputs.len(),
        elapsed_ms,
        runtime_batch_limit,
        rows,
    })
}

fn measure_lens_batch_with_limit(
    registry: &calyx_registry::Registry,
    lens_id: LensId,
    inputs: &[Input],
    runtime_batch_limit: Option<usize>,
) -> calyx_core::Result<Vec<SlotVector>> {
    let Some(limit) = runtime_batch_limit else {
        return registry.measure_batch(lens_id, inputs);
    };
    if limit == 0 {
        return Err(CalyxError::lens_unreachable(
            "resident measure_batch runtime_batch_limit must be > 0 when supplied",
        ));
    }
    let mut out = Vec::with_capacity(inputs.len());
    for chunk in inputs.chunks(limit) {
        out.extend(registry.measure_batch(lens_id, chunk)?);
    }
    Ok(out)
}

fn slot_measure(
    slot: &calyx_core::Slot,
    measured: bool,
    vector: Option<SlotVector>,
    absent_reason: Option<AbsentReason>,
) -> ResidentSlotMeasure {
    ResidentSlotMeasure {
        slot: slot.slot_id.get(),
        key: slot.slot_key.key().to_string(),
        lens_id: slot.lens_id.to_string(),
        modality: slot.modality,
        placement: slot.resource.placement,
        measured,
        vector,
        absent_reason,
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

fn encode_binary(value: &impl Serialize) -> Result<Vec<u8>, CalyxError> {
    bincode::serde::encode_to_vec(value, config::standard()).map_err(|error| CalyxError {
        code: "CALYX_PANEL_RESIDENT_BINARY_ENCODE",
        message: format!("encode resident binary frame failed: {error}"),
        remediation: "restart the resident service from the same Calyx build as the CLI",
    })
}

fn decode_binary<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, CalyxError> {
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

fn write_binary_response(
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

fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> Result<(), CalyxError> {
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

fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>, CalyxError> {
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

fn write_json_file(path: PathBuf, value: &impl Serialize) -> CliResult {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn cli_error_value(error: &CliError) -> Value {
    error_value(error.code(), error.message(), error.remediation())
}

fn error_value(
    code: impl Into<String>,
    message: impl Into<String>,
    remediation: impl Into<String>,
) -> Value {
    json!({
        "ok": false,
        "code": code.into(),
        "message": message.into(),
        "remediation": remediation.into(),
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use calyx_core::Placement;

    use super::*;

    #[test]
    fn provided_home_does_not_evaluate_env_fallback() {
        let home = PathBuf::from(r"C:\calyx");
        let resolved = resolve_home_with(Some(home.clone()), || {
            panic!("explicit --home must not read CALYX_HOME")
        })
        .unwrap();

        assert_eq!(resolved, home);
    }

    #[test]
    fn resident_service_binary_request_roundtrips_without_json_shape() {
        let request = ResidentMeasureBatchBinaryRequest {
            protocol_version: RESIDENT_BINARY_PROTOCOL_VERSION,
            modality: Modality::Text,
            inputs: vec![b"alpha".to_vec(), b"beta".to_vec()],
            runtime_batch_limit: Some(4),
        };

        let bytes = encode_binary(&request).unwrap();
        let decoded: ResidentMeasureBatchBinaryRequest = decode_binary(&bytes).unwrap();
        println!(
            "resident_service_binary_request bytes={} inputs={} runtime_batch_limit={:?}",
            bytes.len(),
            decoded.inputs.len(),
            decoded.runtime_batch_limit
        );

        assert_eq!(decoded.protocol_version, RESIDENT_BINARY_PROTOCOL_VERSION);
        assert_eq!(decoded.modality, Modality::Text);
        assert_eq!(decoded.inputs, request.inputs);
        assert_eq!(decoded.runtime_batch_limit, Some(4));
        let lossy = String::from_utf8_lossy(&bytes);
        assert!(
            !lossy.contains("inputs_hex") && !lossy.contains("runtime_batch_limit"),
            "resident service binary IPC must not carry JSON field names"
        );
    }

    #[test]
    fn resident_service_binary_frame_readback_is_length_prefixed() {
        let response = ResidentMeasureBatchBinaryResponse {
            protocol_version: RESIDENT_BINARY_PROTOCOL_VERSION,
            result: ResidentMeasureBatchBinaryResult::Err {
                code: "CALYX_TEST".to_string(),
                message: "synthetic resident frame edge".to_string(),
                remediation: "fix test input".to_string(),
            },
        };
        let payload = encode_binary(&response).unwrap();
        let mut stream = Cursor::new(Vec::new());

        write_frame(&mut stream, &payload).unwrap();
        let stored = stream.into_inner();
        println!(
            "resident_service_binary_frame_state header_bytes=8 payload_bytes={} stored_bytes={}",
            payload.len(),
            stored.len()
        );

        assert_eq!(stored.len(), payload.len() + 8);
        assert_eq!(
            u64::from_be_bytes(stored[..8].try_into().unwrap()) as usize,
            payload.len()
        );
        let mut readback = Cursor::new(stored);
        let decoded_payload = read_frame(&mut readback).unwrap();
        let decoded: ResidentMeasureBatchBinaryResponse = decode_binary(&decoded_payload).unwrap();
        assert_eq!(decoded.protocol_version, RESIDENT_BINARY_PROTOCOL_VERSION);
        assert!(matches!(
            decoded.result,
            ResidentMeasureBatchBinaryResult::Err { ref code, .. } if code == "CALYX_TEST"
        ));
    }

    #[test]
    fn resident_service_binary_measure_batch_response_roundtrips_with_vectors() {
        let response = ResidentMeasureBatchBinaryResponse {
            protocol_version: RESIDENT_BINARY_PROTOCOL_VERSION,
            result: ResidentMeasureBatchBinaryResult::Ok(MeasureBatchResponse {
                schema: MEASURE_BATCH_SCHEMA.to_string(),
                ready: true,
                process_id: 42,
                template_source: "synthetic-template".to_string(),
                modality: Modality::Text,
                input_count: 1,
                elapsed_ms: 7,
                runtime_batch_limit: Some(4),
                rows: vec![ResidentMeasuredInput {
                    input_index: 0,
                    input_len: 24,
                    measured_slot_count: 1,
                    absent_slot_count: 0,
                    slots: vec![ResidentSlotMeasure {
                        slot: 0,
                        key: "dense".to_string(),
                        lens_id: "00000000000000000000000000000000".to_string(),
                        modality: Modality::Text,
                        placement: Placement::Gpu,
                        measured: true,
                        vector: Some(SlotVector::Dense {
                            dim: 2,
                            data: vec![0.25, 0.75],
                        }),
                        absent_reason: None,
                    }],
                }],
            }),
        };

        let bytes = encode_binary(&response).unwrap();
        println!("resident_service_binary_response bytes={}", bytes.len());
        let decoded: ResidentMeasureBatchBinaryResponse = decode_binary(&bytes).unwrap();

        assert_eq!(decoded.protocol_version, RESIDENT_BINARY_PROTOCOL_VERSION);
        match decoded.result {
            ResidentMeasureBatchBinaryResult::Ok(parsed) => {
                assert_eq!(parsed.schema, MEASURE_BATCH_SCHEMA);
                assert_eq!(parsed.modality, Modality::Text);
                assert_eq!(parsed.rows.len(), 1);
                assert_eq!(parsed.rows[0].slots.len(), 1);
                assert!(matches!(
                    parsed.rows[0].slots[0].vector,
                    Some(SlotVector::Dense { dim: 2, .. })
                ));
            }
            ResidentMeasureBatchBinaryResult::Err { code, message, .. } => {
                panic!("expected ok response, got {code}: {message}");
            }
        }
    }

    #[test]
    fn resident_service_binary_truncated_frame_fails_loud() {
        let mut stream = Cursor::new(Vec::new());
        stream.write_all(&16_u64.to_be_bytes()).unwrap();
        stream.write_all(b"short").unwrap();
        stream.set_position(0);

        let error = read_frame(&mut stream).unwrap_err();
        println!(
            "resident_service_binary_truncated_error code={} message={}",
            error.code, error.message
        );

        assert_eq!(error.code, "CALYX_PANEL_RESIDENT_BINARY_FRAME");
        assert!(
            error
                .message
                .contains("read resident service binary frame body")
        );
    }
}
