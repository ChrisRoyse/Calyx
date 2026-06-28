use super::load::register_and_prime_warm_lenses_parallel;
use super::load_progress::is_content_slot;
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
    let declared_bytes = template
        .lenses
        .iter()
        .map(|lens| lens.cost.vram_bytes)
        .fold(0_u64, u64::saturating_add);
    let declared_template_vram_mib = bytes_to_mib_ceil(declared_bytes);
    let estimated_resident_vram_mib =
        estimate_resident_vram_mib(declared_bytes, resident_overhead_multiplier_milli);
    let semantic_lenses = semantic_lens_count(&template.lenses);
    if let Some(log) = progress_log {
        let mut record = run_progress_record(selector, "vram_preflight");
        record.lens_count = Some(template.lenses.len());
        record.semantic_lens_count = Some(semantic_lenses);
        record.declared_template_vram_mib = Some(declared_template_vram_mib);
        record.estimated_resident_vram_mib = Some(estimated_resident_vram_mib);
        record.max_resident_vram_mib = Some(max_resident_vram_mib);
        record.resident_overhead_multiplier =
            Some(multiplier_to_f32(resident_overhead_multiplier_milli));
        log.append(&record)?;
    }
    if estimated_resident_vram_mib > max_resident_vram_mib {
        let message = format!(
            "panel warm refuses template {selector}: declared_vram_mib={declared_template_vram_mib} \
             resident_overhead_multiplier={} estimated_resident_vram_mib={estimated_resident_vram_mib} \
             max_resident_vram_mib={max_resident_vram_mib} lens_count={} semantic_lens_count={semantic_lenses}; \
             this would exceed the Blackwell RTX 5090 22GiB resident target before inference",
            multiplier_to_f32(resident_overhead_multiplier_milli),
            template.lenses.len(),
        );
        if let Some(log) = progress_log {
            let mut record = run_progress_record(selector, "vram_preflight_error");
            record.lens_count = Some(template.lenses.len());
            record.semantic_lens_count = Some(semantic_lenses);
            record.declared_template_vram_mib = Some(declared_template_vram_mib);
            record.estimated_resident_vram_mib = Some(estimated_resident_vram_mib);
            record.max_resident_vram_mib = Some(max_resident_vram_mib);
            record.resident_overhead_multiplier =
                Some(multiplier_to_f32(resident_overhead_multiplier_milli));
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
        lens_count: template.lenses.len(),
        declared_template_vram_mib,
        estimated_resident_vram_mib,
    })
}

pub(super) fn default_load_parallelism(lens_count: usize) -> usize {
    lens_count.clamp(1, DEFAULT_LOAD_PARALLELISM)
}

fn semantic_lens_count(lenses: &[template_store::TemplateLensRef]) -> usize {
    lenses
        .iter()
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
    let mut template = store.load(selector)?;
    template.validate()?;
    let a37 = template.a37_admission();
    let template_id = template_store::id_for_loaded(&template)?;
    let mut registry = Registry::new();
    let registered_lenses_added = register_and_prime_warm_lenses_parallel(
        &mut registry,
        &mut template,
        progress_log,
        selector,
        load_limit,
        load_parallelism,
    )?;
    let panel = template.to_target_panel(now_ms);
    let content_lens_count = a37.content_lens_count.max(
        panel
            .slots
            .iter()
            .filter(|slot| is_content_slot(slot))
            .count(),
    );
    Ok(SavedTemplatePanelBuild {
        template_id,
        template_name: template.name,
        panel,
        registry,
        content_lens_count,
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

fn bytes_to_mib_ceil(bytes: u64) -> u64 {
    bytes.saturating_add((1024 * 1024) - 1) / (1024 * 1024)
}

pub(super) fn multiplier_to_f32(multiplier_milli: u64) -> f32 {
    multiplier_milli as f32 / 1000.0
}
