use super::*;

pub(super) fn probe_panel(
    build: &SavedTemplatePanelBuild,
    progress_log: Option<&WarmProgressLog>,
    template: &str,
) -> CliResult<Vec<WarmProbeReport>> {
    let mut seen = BTreeSet::new();
    let mut reports = Vec::new();
    let slots = content_slots(build).collect::<Vec<_>>();
    let total = slots
        .iter()
        .map(|slot| slot.lens_id)
        .collect::<BTreeSet<_>>()
        .len();
    let mut ordinal = 0;
    for slot in slots {
        if !seen.insert(slot.lens_id) {
            continue;
        }
        ordinal += 1;
        let spec = build.registry.lens_spec(slot.lens_id).ok_or_else(|| {
            CliError::from(CalyxError::registry_unavailable(format!(
                "warm probe slot={} key={} lens={} has no LensSpec in registry",
                slot.slot_id.get(),
                slot.slot_key.key(),
                slot.lens_id
            )))
        })?;
        if let Some(log) = progress_log {
            log.append(&probe_progress_record(
                template,
                ProbeProgressEvent {
                    phase: "probe_start",
                    ordinal,
                    total,
                    slot,
                    spec_name: spec.name.as_str(),
                    runtime: &spec.runtime,
                    elapsed_ms: None,
                    error: None,
                },
            ))?;
        }
        let input = Input::new(slot.modality, probe_bytes(slot.modality)?);
        let started = Instant::now();
        let vector = match build.registry.measure(slot.lens_id, &input) {
            Ok(vector) => vector,
            Err(error) => {
                if let Some(log) = progress_log {
                    log.append(&probe_progress_record(
                        template,
                        ProbeProgressEvent {
                            phase: "probe_error",
                            ordinal,
                            total,
                            slot,
                            spec_name: spec.name.as_str(),
                            runtime: &spec.runtime,
                            elapsed_ms: Some(started.elapsed().as_millis()),
                            error: Some((error.code, error.message.as_str())),
                        },
                    ))?;
                }
                return Err(warm_error(slot, spec.name.as_str(), &spec.runtime, error));
            }
        };
        if let Err(error) = validate_vector_contract(&vector, slot.shape, spec.norm_policy) {
            if let Some(log) = progress_log {
                log.append(&probe_progress_record(
                    template,
                    ProbeProgressEvent {
                        phase: "probe_error",
                        ordinal,
                        total,
                        slot,
                        spec_name: spec.name.as_str(),
                        runtime: &spec.runtime,
                        elapsed_ms: Some(started.elapsed().as_millis()),
                        error: Some((error.code(), error.message())),
                    },
                ))?;
            }
            return Err(warm_cli_error(
                slot,
                spec.name.as_str(),
                &spec.runtime,
                error,
            ));
        }
        let report = report_probe(
            slot,
            spec.name.as_str(),
            &spec.runtime,
            &vector,
            started.elapsed().as_millis(),
        );
        if let Some(log) = progress_log {
            log.append(&probe_ok_record(
                template,
                ordinal,
                total,
                slot,
                spec.name.as_str(),
                &spec.runtime,
                &report,
            ))?;
        }
        reports.push(report);
    }
    Ok(reports)
}

fn report_probe(
    slot: &Slot,
    spec_name: &str,
    runtime: &LensRuntime,
    vector: &SlotVector,
    elapsed_ms: u128,
) -> WarmProbeReport {
    let (vector_kind, vector_len) = vector_kind_len(vector);
    WarmProbeReport {
        slot: slot.slot_id.get(),
        key: slot.slot_key.key().to_string(),
        lens_id: slot.lens_id.to_string(),
        spec_name: spec_name.to_string(),
        runtime: runtime_name(runtime),
        runtime_detail: runtime_detail(runtime),
        modality: slot.modality,
        shape: slot.shape,
        placement: slot.resource.placement,
        vector_kind,
        vector_len,
        norm: slot_norm(vector),
        first_values: slot_prefix(vector, 4),
        elapsed_ms,
    }
}

pub(super) fn run_progress_record(template: &str, phase: &str) -> WarmProgressRecord {
    base_progress_record(template, phase)
}

pub(super) fn registration_progress_record(
    template: &str,
    event: TemplateLensProgress,
) -> WarmProgressRecord {
    let mut record = base_progress_record(template, event.phase);
    record.ordinal = Some(event.ordinal);
    record.total = Some(event.total);
    record.key = Some(event.slot_key);
    record.lens_id = Some(event.lens_id);
    record.runtime_lens_id = event.runtime_lens_id;
    record.lens_name = Some(event.lens_name);
    record.runtime = Some(event.runtime);
    record.modality = Some(event.modality);
    record.shape = Some(event.shape);
    record.placement = Some(event.placement);
    record.manifest = Some(event.manifest);
    record
}

struct ProbeProgressEvent<'a> {
    phase: &'a str,
    ordinal: usize,
    total: usize,
    slot: &'a Slot,
    spec_name: &'a str,
    runtime: &'a LensRuntime,
    elapsed_ms: Option<u128>,
    error: Option<(&'a str, &'a str)>,
}

fn probe_progress_record(template: &str, event: ProbeProgressEvent<'_>) -> WarmProgressRecord {
    let mut record = slot_progress_record(
        template,
        event.phase,
        event.ordinal,
        event.total,
        event.slot,
        event.spec_name,
        event.runtime,
    );
    record.elapsed_ms = event.elapsed_ms;
    if let Some((code, message)) = event.error {
        record.error_code = Some(code.to_string());
        record.error_message = Some(message.to_string());
    }
    record
}

fn probe_ok_record(
    template: &str,
    ordinal: usize,
    total: usize,
    slot: &Slot,
    spec_name: &str,
    runtime: &LensRuntime,
    report: &WarmProbeReport,
) -> WarmProgressRecord {
    let mut record = slot_progress_record(
        template, "probe_ok", ordinal, total, slot, spec_name, runtime,
    );
    record.elapsed_ms = Some(report.elapsed_ms);
    record.vector_kind = Some(report.vector_kind);
    record.vector_len = Some(report.vector_len);
    record.norm = Some(report.norm);
    record.first_values = Some(report.first_values.clone());
    record
}

fn slot_progress_record(
    template: &str,
    phase: &str,
    ordinal: usize,
    total: usize,
    slot: &Slot,
    spec_name: &str,
    runtime: &LensRuntime,
) -> WarmProgressRecord {
    let mut record = base_progress_record(template, phase);
    record.ordinal = Some(ordinal);
    record.total = Some(total);
    record.slot = Some(slot.slot_id.get());
    record.key = Some(slot.slot_key.key().to_string());
    record.lens_id = Some(slot.lens_id.to_string());
    record.spec_name = Some(spec_name.to_string());
    record.runtime = Some(runtime_name(runtime).to_string());
    record.runtime_detail = Some(runtime_detail(runtime));
    record.modality = Some(format!("{:?}", slot.modality));
    record.shape = Some(format!("{:?}", slot.shape));
    record.placement = Some(format!("{:?}", slot.resource.placement));
    record
}

pub(super) fn base_progress_record(template: &str, phase: &str) -> WarmProgressRecord {
    WarmProgressRecord {
        schema: PROGRESS_SCHEMA,
        timestamp_ms: now_ms(),
        process_id: std::process::id(),
        template_selector: template.to_string(),
        phase: phase.to_string(),
        ordinal: None,
        total: None,
        slot: None,
        key: None,
        lens_id: None,
        runtime_lens_id: None,
        lens_name: None,
        spec_name: None,
        runtime: None,
        runtime_detail: None,
        modality: None,
        shape: None,
        placement: None,
        manifest: None,
        elapsed_ms: None,
        lens_count: None,
        semantic_lens_count: None,
        declared_template_vram_mib: None,
        estimated_resident_vram_mib: None,
        max_resident_vram_mib: None,
        resident_overhead_multiplier: None,
        load_parallelism: None,
        vector_kind: None,
        vector_len: None,
        norm: None,
        first_values: None,
        error_code: None,
        error_message: None,
        remediation: None,
    }
}

pub(super) fn content_slots(build: &SavedTemplatePanelBuild) -> impl Iterator<Item = &Slot> {
    build.panel.slots.iter().filter(|slot| {
        slot.state == SlotState::Active && !slot.retrieval_only && !slot.excluded_from_dedup
    })
}

pub(super) fn vector_kind_len(vector: &SlotVector) -> (&'static str, usize) {
    match vector {
        SlotVector::Dense { data, .. } => ("dense", data.len()),
        SlotVector::Sparse { entries, .. } => ("sparse", entries.len()),
        SlotVector::Multi { tokens, .. } => ("multi", tokens.len()),
        SlotVector::Absent { .. } => ("absent", 0),
    }
}

pub(super) fn probe_bytes(modality: Modality) -> CliResult<Vec<u8>> {
    match modality {
        Modality::Text => Ok(b"Calyx Blackwell warm-load probe: semantic text path.".to_vec()),
        Modality::Code => Ok(b"fn calyx_warm_probe() -> u32 { 42 }".to_vec()),
        Modality::Image => Ok(one_pixel_png().to_vec()),
        Modality::Audio => Ok(warm_audio_wav()),
        Modality::Video => Ok(b"RIFF\x24\x00\x00\x00AVI LIST calyx warm probe".to_vec()),
        Modality::Protein => Ok(b"MKTFFVLLL".to_vec()),
        Modality::Dna => Ok(b"ACGTNACGTN".to_vec()),
        Modality::Molecule => Ok(b"CCO".to_vec()),
        Modality::Structured => Ok(br#"{"calyx_warm_probe":true,"value":42}"#.to_vec()),
        Modality::Mixed => Ok(b"Calyx mixed modality warm probe with text and metadata.".to_vec()),
    }
}

fn warm_error(slot: &Slot, spec_name: &str, runtime: &LensRuntime, error: CalyxError) -> CliError {
    CliError::from(CalyxError {
        code: error.code,
        message: warm_error_message(slot, spec_name, runtime, error.code, &error.message),
        remediation: error.remediation,
    })
}

fn warm_cli_error(
    slot: &Slot,
    spec_name: &str,
    runtime: &LensRuntime,
    error: CliError,
) -> CliError {
    CliError::from(CalyxError {
        code: error.code(),
        message: warm_error_message(slot, spec_name, runtime, error.code(), error.message()),
        remediation: error.remediation(),
    })
}

fn warm_error_message(
    slot: &Slot,
    spec_name: &str,
    runtime: &LensRuntime,
    code: &str,
    message: &str,
) -> String {
    format!(
        "panel warm failed slot={} key={} lens={} spec_name={} runtime={} runtime_detail={} modality={:?} shape={:?} placement={:?}; cause_code={code}; cause={message}",
        slot.slot_id.get(),
        slot.slot_key.key(),
        slot.lens_id,
        spec_name,
        runtime_name(runtime),
        runtime_detail(runtime),
        slot.modality,
        slot.shape,
        slot.resource.placement,
    )
}

pub(super) fn runtime_detail(runtime: &LensRuntime) -> String {
    match runtime {
        LensRuntime::Algorithmic { kind } => kind.clone(),
        LensRuntime::TeiHttp { endpoint } => endpoint.clone(),
        LensRuntime::CandleLocal {
            model_id,
            dtype,
            pooling,
            ..
        } => format!("{model_id};dtype={dtype};pooling={pooling}"),
        LensRuntime::Onnx { model_id, .. }
        | LensRuntime::OnnxColbert { model_id, .. }
        | LensRuntime::FastembedSparse { model_id, .. }
        | LensRuntime::FastembedReranker { model_id, .. } => model_id.clone(),
        LensRuntime::FastembedBgem3 {
            model_id, output, ..
        } => format!("{model_id};output={output:?}"),
        LensRuntime::FastembedQwen3 {
            model_id, dtype, ..
        } => format!("{model_id};dtype={dtype}"),
        LensRuntime::StaticLookup {
            embeddings_file,
            tokenizer,
            ..
        } => format!(
            "embeddings={};tokenizer={}",
            embeddings_file.display(),
            tokenizer.display()
        ),
        LensRuntime::MultimodalAdapter {
            axis,
            model_id,
            adapter_config,
            ..
        } => format!(
            "axis={axis};model_id={model_id};adapter_config={}",
            adapter_config
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "missing".to_string())
        ),
        LensRuntime::ExternalCmd { cmd, args } => format!("{cmd} {}", args.join(" ")),
    }
}
