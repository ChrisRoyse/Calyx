use super::load::WarmLensTask;
use super::probe::{base_progress_record, registration_progress_record, runtime_detail};
use super::*;

pub(super) fn append_shared_progress(
    progress_log: &SharedProgressLog,
    record: &WarmProgressRecord,
) -> CliResult {
    let Some(log) = progress_log else {
        return Ok(());
    };
    let log = log.lock().map_err(|_| {
        CliError::from(CalyxError::lens_unreachable(
            "warm progress log mutex was poisoned",
        ))
    })?;
    log.append(record)
}

pub(super) fn task_progress_record(
    selector: &str,
    phase: &'static str,
    task: &WarmLensTask,
) -> WarmProgressRecord {
    registration_progress_record(
        selector,
        TemplateLensProgress {
            phase,
            ordinal: task.position,
            total: task.total,
            slot_key: task.lens.slot_key.clone(),
            lens_name: task.lens.lens_name.clone(),
            lens_id: task.lens.lens_id.to_string(),
            runtime_lens_id: task.lens.runtime_lens_id.map(|id| id.to_string()),
            runtime: task.lens.runtime.clone(),
            modality: format!("{:?}", task.lens.modality),
            shape: format!("{:?}", task.lens.shape),
            placement: format!("{:?}", task.lens.placement),
            manifest: task.lens.manifest.clone(),
        },
    )
}

pub(super) fn emit_registration_progress_shared(
    progress_log: &SharedProgressLog,
    selector: &str,
    phase: &'static str,
    ordinal: usize,
    total: usize,
    lens: &template_store::TemplateLensRef,
    elapsed_ms: Option<u128>,
) -> CliResult {
    let mut record = registration_progress_record(
        selector,
        TemplateLensProgress {
            phase,
            ordinal,
            total,
            slot_key: lens.slot_key.clone(),
            lens_name: lens.lens_name.clone(),
            lens_id: lens.lens_id.to_string(),
            runtime_lens_id: lens.runtime_lens_id.map(|id| id.to_string()),
            runtime: lens.runtime.clone(),
            modality: format!("{:?}", lens.modality),
            shape: format!("{:?}", lens.shape),
            placement: format!("{:?}", lens.placement),
            manifest: lens.manifest.clone(),
        },
    );
    record.elapsed_ms = elapsed_ms;
    append_shared_progress(progress_log, &record)
}

pub(super) fn prime_progress_record(
    template: &str,
    phase: &str,
    ordinal: usize,
    total: usize,
    lens: &template_store::TemplateLensRef,
    runtime_lens_id: calyx_core::LensId,
    runtime: &LensRuntime,
) -> WarmProgressRecord {
    let mut record = base_progress_record(template, phase);
    record.ordinal = Some(ordinal);
    record.total = Some(total);
    record.key = Some(lens.slot_key.clone());
    record.lens_id = Some(lens.lens_id.to_string());
    record.runtime_lens_id = Some(runtime_lens_id.to_string());
    record.lens_name = Some(lens.lens_name.clone());
    record.runtime = Some(runtime_name(runtime).to_string());
    record.runtime_detail = Some(runtime_detail(runtime));
    record.modality = Some(format!("{:?}", lens.modality));
    record.shape = Some(format!("{:?}", lens.shape));
    record.placement = Some(format!("{:?}", lens.placement));
    record.manifest = Some(lens.manifest.clone());
    record
}

pub(super) struct PrimeErrorEvent {
    pub(super) elapsed_ms: u128,
    pub(super) error_code: String,
    pub(super) error_message: String,
}

pub(super) fn prime_error_record(
    template: &str,
    ordinal: usize,
    total: usize,
    lens: &template_store::TemplateLensRef,
    runtime_lens_id: calyx_core::LensId,
    runtime: &LensRuntime,
    error: PrimeErrorEvent,
) -> WarmProgressRecord {
    let mut record = prime_progress_record(
        template,
        "prime_error",
        ordinal,
        total,
        lens,
        runtime_lens_id,
        runtime,
    );
    record.elapsed_ms = Some(error.elapsed_ms);
    record.error_code = Some(error.error_code);
    record.error_message = Some(error.error_message);
    record
}

pub(super) fn warm_prime_error(
    lens: &template_store::TemplateLensRef,
    spec_name: &str,
    runtime: &LensRuntime,
    error: CalyxError,
) -> CliError {
    CliError::from(CalyxError::lens_unreachable(format!(
        "panel warm prime failed key={} lens={} spec_name={} runtime={} runtime_detail={} \
         modality={:?} shape={:?} placement={:?}; cause_code={}; cause={}",
        lens.slot_key,
        lens.lens_id,
        spec_name,
        runtime_name(runtime),
        runtime_detail(runtime),
        lens.modality,
        lens.shape,
        lens.placement,
        error.code,
        error.message
    )))
}

pub(super) fn warm_prime_cli_error(
    lens: &template_store::TemplateLensRef,
    spec_name: &str,
    runtime: &LensRuntime,
    error: CliError,
) -> CliError {
    CliError::from(CalyxError::lens_unreachable(format!(
        "panel warm prime failed key={} lens={} spec_name={} runtime={} runtime_detail={} \
         modality={:?} shape={:?} placement={:?}; cause_code={}; cause={}",
        lens.slot_key,
        lens.lens_id,
        spec_name,
        runtime_name(runtime),
        runtime_detail(runtime),
        lens.modality,
        lens.shape,
        lens.placement,
        error.code(),
        error.message()
    )))
}

pub(super) fn is_content_slot(slot: &Slot) -> bool {
    slot.state == SlotState::Active && slot.modality != Modality::Structured
}
