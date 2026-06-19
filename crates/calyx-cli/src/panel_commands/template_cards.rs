use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use calyx_registry::{CapabilityCard, LensHealth};

use super::template_model::{
    CARD_VERSION, CapabilityCardRef, SavedPanelTemplate, TEMPLATE_INVALID, TemplateEnsembleCard,
    template_error,
};
use crate::error::CliResult;

pub(super) fn ensemble_card_from_capability_cards(
    template: &SavedPanelTemplate,
    card_paths: &[PathBuf],
) -> CliResult<TemplateEnsembleCard> {
    let mut refs = Vec::new();
    for path in card_paths {
        let bytes = fs::read(path)?;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let card: CapabilityCard = serde_json::from_slice(&bytes)?;
        refs.push(CapabilityCardRef {
            path: path.display().to_string(),
            blake3_hex: hash,
            lens_id: card.lens_id,
            probe_count: card.probe_count,
            coverage_rate: card.coverage.rate,
            failed: card.coverage.failed,
            health: card.health,
        });
    }
    ensure_all_template_lenses_measured(template, &refs)?;
    let min_coverage_rate = refs
        .iter()
        .map(|item| item.coverage_rate)
        .fold(f32::INFINITY, f32::min);
    Ok(TemplateEnsembleCard {
        schema_version: CARD_VERSION,
        source: "capability_card_rollup_v1".to_string(),
        content_lens_count: template.content_lens_count(),
        measured_lens_count: refs.len(),
        all_loaded: refs
            .iter()
            .all(|item| matches!(item.health, LensHealth::Loaded)),
        min_coverage_rate,
        total_vram_bytes: sum_vram(template),
        total_ram_bytes: sum_ram(template),
        mean_ms_per_input: mean_ms(template),
        card_refs: refs,
    })
}

fn ensure_all_template_lenses_measured(
    template: &SavedPanelTemplate,
    refs: &[CapabilityCardRef],
) -> CliResult {
    let wanted: BTreeSet<_> = template.lenses.iter().map(|lens| lens.lens_id).collect();
    let measured: BTreeSet<_> = refs.iter().map(|item| item.lens_id).collect();
    let missing = wanted.difference(&measured).collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    Err(template_error(
        TEMPLATE_INVALID,
        format!(
            "missing capability cards for {} template lenses",
            missing.len()
        ),
        "profile every content lens in the template before saving the ensemble card",
    ))
}

fn sum_vram(template: &SavedPanelTemplate) -> u64 {
    template
        .lenses
        .iter()
        .map(|lens| lens.cost.vram_bytes)
        .fold(0_u64, u64::saturating_add)
}

fn sum_ram(template: &SavedPanelTemplate) -> u64 {
    template
        .lenses
        .iter()
        .map(|lens| lens.cost.ram_bytes)
        .fold(0_u64, u64::saturating_add)
}

fn mean_ms(template: &SavedPanelTemplate) -> f32 {
    if template.lenses.is_empty() {
        return 0.0;
    }
    template
        .lenses
        .iter()
        .map(|lens| lens.cost.ms_per_input)
        .sum::<f32>()
        / template.lenses.len() as f32
}
