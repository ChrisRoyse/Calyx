use super::codec::{decode_binary, read_frame};
use super::server::ResidentService;
use super::*;

pub(crate) fn dispatch_binary_measure_batch(
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
