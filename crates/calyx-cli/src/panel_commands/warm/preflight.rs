use super::load::{
    register_and_prime_scoped_warm_lenses_parallel, register_and_prime_warm_lenses_parallel,
};
use super::probe::run_progress_record;
use super::*;

pub(super) fn warm_preflight(
    home: &Path,
    selector: &str,
    max_resident_vram_mib: u64,
    resident_overhead_multiplier_milli: u64,
    progress_log: Option<&WarmProgressLog>,
) -> CliResult<WarmPreflight> {
    let store = template_store::TemplateStore::open(home);
    let template = store.load(selector)?;
    template.validate()?;
    warm_preflight_loaded(
        selector,
        &template,
        None,
        max_resident_vram_mib,
        resident_overhead_multiplier_milli,
        progress_log,
    )
}

pub(super) fn warm_preflight_loaded(
    selector: &str,
    template: &template_store::SavedPanelTemplate,
    selected_slots: Option<&BTreeSet<SlotId>>,
    max_resident_vram_mib: u64,
    resident_overhead_multiplier_milli: u64,
    progress_log: Option<&WarmProgressLog>,
) -> CliResult<WarmPreflight> {
    let selected_lenses = template
        .lenses
        .iter()
        .enumerate()
        .filter(|(template_idx, _)| {
            selected_slots.is_none_or(|slots| {
                u16::try_from(*template_idx)
                    .ok()
                    .is_some_and(|slot| slots.contains(&SlotId::new(slot)))
            })
        })
        .map(|(_, lens)| lens)
        .collect::<Vec<_>>();
    let declared_bytes = selected_lenses.iter().try_fold(0_u64, |total, lens| {
        total.checked_add(lens.cost.vram_bytes).ok_or_else(|| {
            CliError::from(CalyxError {
                code: WARM_VRAM_BUDGET,
                message: format!(
                    "panel warm template {selector} declared VRAM byte sum overflowed u64: accumulated_bytes={total} next_runtime_bytes={}",
                    lens.cost.vram_bytes
                ),
                remediation:
                    "repair the immutable template lens costs; declared VRAM costs must sum exactly within u64",
            })
        })
    })?;
    let declared_template_vram_mib = bytes_to_mib_ceil(declared_bytes);
    let estimated_resident_vram_mib =
        estimate_resident_vram_mib(declared_bytes, resident_overhead_multiplier_milli);
    let semantic_lenses = semantic_lens_count(selected_lenses.iter().copied());
    if let Some(log) = progress_log {
        let mut record = run_progress_record(selector, "vram_preflight");
        record.lens_count = Some(selected_lenses.len());
        record.semantic_lens_count = Some(semantic_lenses);
        record.declared_template_vram_mib = Some(declared_template_vram_mib);
        record.estimated_resident_vram_mib = Some(estimated_resident_vram_mib);
        record.max_resident_vram_mib = Some(max_resident_vram_mib);
        record.resident_overhead_multiplier_milli = Some(resident_overhead_multiplier_milli);
        log.append(&record)?;
    }
    if estimated_resident_vram_mib > max_resident_vram_mib {
        let message = format!(
            "panel warm refuses template {selector}: declared_vram_mib={declared_template_vram_mib} \
             resident_overhead_multiplier={} estimated_resident_vram_mib={estimated_resident_vram_mib} \
             max_resident_vram_mib={max_resident_vram_mib} lens_count={} semantic_lens_count={semantic_lenses}; \
             this would exceed the Blackwell RTX 5090 22GiB resident target before inference",
            format_multiplier_milli(resident_overhead_multiplier_milli),
            selected_lenses.len(),
        );
        if let Some(log) = progress_log {
            let mut record = run_progress_record(selector, "vram_preflight_error");
            record.lens_count = Some(selected_lenses.len());
            record.semantic_lens_count = Some(semantic_lenses);
            record.declared_template_vram_mib = Some(declared_template_vram_mib);
            record.estimated_resident_vram_mib = Some(estimated_resident_vram_mib);
            record.max_resident_vram_mib = Some(max_resident_vram_mib);
            record.resident_overhead_multiplier_milli = Some(resident_overhead_multiplier_milli);
            record.error_code = Some(WARM_VRAM_BUDGET.to_string());
            record.error_message = Some(message.clone());
            record.remediation = Some(
                "prune duplicate semantic embedders to 2-3 compact GPU/INT8 choices, replace \
                 non-Blackwell-friendly lenses, or raise the explicit --max-resident-vram-mib"
                    .to_string(),
            );
            log.append(&record)?;
        }
        return Err(CliError::from(CalyxError {
            code: WARM_VRAM_BUDGET,
            message,
            remediation: "prune duplicate semantic embedders to 2-3 compact GPU/INT8 choices, replace non-Blackwell-friendly lenses, or raise the explicit --max-resident-vram-mib",
        }));
    }
    Ok(WarmPreflight {
        lens_count: selected_lenses.len(),
        declared_template_vram_mib,
        estimated_resident_vram_mib,
    })
}

pub(super) fn default_load_parallelism(lens_count: usize) -> usize {
    lens_count.clamp(1, DEFAULT_LOAD_PARALLELISM)
}

fn semantic_lens_count<'a>(
    lenses: impl IntoIterator<Item = &'a template_store::TemplateLensRef>,
) -> usize {
    lenses
        .into_iter()
        .filter(|lens| {
            lens.modality == Modality::Text
                && (lens.slot_key.contains("semantic")
                    || lens.lens_name.contains("semantic")
                    || lens.lens_name.contains("bge")
                    || lens.lens_name.contains("embedding")
                    || lens.lens_name.contains("colbert"))
        })
        .count()
}

pub(super) fn build_warm_template_panel(
    home: &Path,
    selector: &str,
    now_ms: u64,
    progress_log: &SharedProgressLog,
    load_limit: &WarmLoadLimit,
    load_parallelism: usize,
) -> CliResult<SavedTemplatePanelBuild> {
    let store = template_store::TemplateStore::open(home);
    let template = store.load(selector)?;
    template.validate()?;
    let template_id = template_store::id_for_loaded(&template)?;
    build_loaded_warm_template_panel(
        template,
        template_id,
        None,
        selector,
        now_ms,
        progress_log,
        load_limit,
        load_parallelism,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_loaded_warm_template_panel(
    mut template: template_store::SavedPanelTemplate,
    template_id: String,
    selected_slots: Option<&BTreeSet<SlotId>>,
    selector: &str,
    now_ms: u64,
    progress_log: &SharedProgressLog,
    load_limit: &WarmLoadLimit,
    load_parallelism: usize,
) -> CliResult<SavedTemplatePanelBuild> {
    let a37 = template.a37_admission();
    let mut registry = Registry::new();
    let registered_lenses_added = match selected_slots {
        Some(selected_slots) => register_and_prime_scoped_warm_lenses_parallel(
            &mut registry,
            &mut template,
            selected_slots,
            progress_log,
            selector,
            load_limit,
            load_parallelism,
        )?,
        None => register_and_prime_warm_lenses_parallel(
            &mut registry,
            &mut template,
            progress_log,
            selector,
            load_limit,
            load_parallelism,
        )?,
    };
    let panel = template.to_target_panel(now_ms);
    Ok(SavedTemplatePanelBuild {
        template_id,
        template_name: template.name,
        panel,
        registry,
        a37_gate_eligible: a37.gate_eligible,
        a37_status: a37.status,
        registered_lenses_added,
    })
}

pub(super) fn estimate_resident_vram_mib(declared_bytes: u64, multiplier_milli: u64) -> u64 {
    let adjusted = (declared_bytes as u128).saturating_mul(multiplier_milli as u128);
    let adjusted_bytes = adjusted.saturating_add(999) / 1000;
    let mib = 1024_u128 * 1024_u128;
    ((adjusted_bytes.saturating_add(mib - 1)) / mib) as u64
}

pub(super) fn bytes_to_mib_ceil(bytes: u64) -> u64 {
    bytes.saturating_add((1024 * 1024) - 1) / (1024 * 1024)
}

pub(super) fn format_multiplier_milli(multiplier_milli: u64) -> String {
    format!("{}.{:03}", multiplier_milli / 1000, multiplier_milli % 1000)
}
