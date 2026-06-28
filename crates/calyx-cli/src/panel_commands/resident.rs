use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use calyx_core::{AbsentReason, CalyxError, Input, Modality, SlotState, SlotVector};
use serde::Serialize;
use serde_json::{Value, json};

mod flags;
mod protocol;

use flags::{
    ServeFlags, calyx_home, ensure_loopback, parse_addr, parse_client_flags, parse_serve_flags,
};
use protocol::{
    ClientMeasureInput, MEASURE_SCHEMA, MeasureResponse, READY_SCHEMA, ReadyResponse,
    ResidentRequest, ResidentSlotMeasure, hex_decode,
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
    let mut line = String::new();
    {
        let mut reader = BufReader::new(stream.try_clone()?);
        reader.read_line(&mut line)?;
    }
    let response = match serde_json::from_str::<ResidentRequest>(&line) {
        Ok(request) => dispatch_request(request, &service, &running),
        Err(error) => error_value(
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            format!("decode resident request JSON line: {error}"),
            "send one JSON object per connection with op=ready, measure, or shutdown",
        ),
    };
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}

fn dispatch_request(
    request: ResidentRequest,
    service: &ResidentService,
    running: &AtomicBool,
) -> Value {
    match request.op.as_str() {
        "ready" => json!(readiness(service)),
        "measure" => dispatch_measure(request, service),
        "shutdown" => {
            running.store(false, Ordering::SeqCst);
            json!({"ok": true, "schema": READY_SCHEMA, "ready": false, "stopping": true})
        }
        other => error_value(
            "CALYX_PANEL_RESIDENT_BAD_REQUEST",
            format!("unknown resident op {other}"),
            "send op=ready, measure, or shutdown",
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

fn readiness(service: &ResidentService) -> ReadyResponse {
    let state = &service.state;
    ReadyResponse {
        schema: READY_SCHEMA,
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
        schema: MEASURE_SCHEMA,
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
}
