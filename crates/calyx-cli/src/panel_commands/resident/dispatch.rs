use super::server::ResidentService;
use super::*;

pub(crate) fn dispatch_request(
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

pub(crate) fn readiness(service: &ResidentService) -> ReadyResponse {
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
        slot_scope: state.slot_scope.iter().map(|slot| slot.get()).collect(),
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
    // #1153: single-input measure fans out across slots exactly like the
    // batch path — one warm panel walk, all runnable lenses concurrent.
    let measured_by_lens = super::parallel::measure_chunk_lenses(
        service,
        modality,
        std::slice::from_ref(&input),
        None,
    )?;
    let row = super::stream::assemble_row(service, modality, &measured_by_lens, 0, 0, &input)?;
    Ok(MeasureResponse {
        schema: MEASURE_SCHEMA.to_string(),
        ready: true,
        process_id: std::process::id(),
        template_source: service.state.template_source.clone(),
        modality,
        input_len: row.input_len,
        elapsed_ms: started.elapsed().as_millis(),
        measured_slot_count: row.measured_slot_count,
        absent_slot_count: row.absent_slot_count,
        slots: row.slots,
    })
}

fn measure_batch(
    service: &ResidentService,
    modality: Modality,
    input_bytes: Vec<Vec<u8>>,
    runtime_batch_limit: Option<usize>,
) -> CliResult<MeasureBatchResponse> {
    let input_count = input_bytes.len();
    let mut rows = Vec::with_capacity(input_count);
    let elapsed_ms = super::stream::measure_batch_chunked(
        service,
        modality,
        input_bytes,
        runtime_batch_limit,
        &mut |chunk_rows| {
            rows.extend(chunk_rows);
            Ok(())
        },
    )?;
    Ok(MeasureBatchResponse {
        schema: MEASURE_BATCH_SCHEMA.to_string(),
        ready: true,
        process_id: std::process::id(),
        template_source: service.state.template_source.clone(),
        modality,
        input_count,
        elapsed_ms,
        runtime_batch_limit,
        rows,
    })
}

pub(super) fn slot_measure(
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
