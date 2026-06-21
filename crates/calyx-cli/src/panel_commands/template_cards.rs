use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{A37_DIVERSITY_GATE_PASSED, EnsembleCard, a37_association_family};
use calyx_registry::{CapabilityCard, LensHealth};
use serde::Deserialize;

use super::template_model::{
    A37_ADMISSION_VERSION, CARD_VERSION, CapabilityCardRef, SavedPanelTemplate, TEMPLATE_INVALID,
    TemplateA37Admission, TemplateA37CardRef, TemplateEnsembleCard, template_error,
};
use crate::error::CliResult;

pub(super) fn ensemble_card_from_capability_cards(
    template: &SavedPanelTemplate,
    card_paths: &[PathBuf],
    a37_card_path: Option<&Path>,
    a37_admission_card_path: Option<&Path>,
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
    let (mut a37_admission, a37_ensemble_card_ref) =
        a37_admission_from_assay_card(template, a37_card_path)?;
    let a37_admission_card_ref = if let Some(path) = a37_admission_card_path {
        let (admission, card_ref) = a37_admission_from_multi_anchor_card(template, path)?;
        a37_admission = admission;
        Some(card_ref)
    } else {
        None
    };
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
        a37_admission,
        a37_ensemble_card_ref,
        a37_admission_card_ref,
    })
}

#[derive(Deserialize)]
struct MultiAnchorAdmissionCard {
    schema_version: u32,
    role: String,
    status: String,
    gate_passed: bool,
    lens_count: usize,
    association_family_count: usize,
    family_span_pass: bool,
    redundancy_bound_pass: bool,
    no_collapse_pass: bool,
    lenses: Vec<MultiAnchorLens>,
}

#[derive(Deserialize)]
struct MultiAnchorLens {
    name: String,
}

fn a37_admission_from_assay_card(
    template: &SavedPanelTemplate,
    path: Option<&Path>,
) -> CliResult<(TemplateA37Admission, Option<TemplateA37CardRef>)> {
    let Some(path) = path else {
        return Ok((missing_a37_admission(template), None));
    };
    let bytes = fs::read(path)?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let card = serde_json::from_slice::<EnsembleCard>(&bytes).map_err(|error| {
        template_error(
            TEMPLATE_INVALID,
            format!(
                "A37 assay card {} is not an EnsembleCard: {error}",
                path.display()
            ),
            "pass the JSON produced by calyx assay i8bin-ensemble-card",
        )
    })?;
    ensure_a37_card_matches_template(template, &card, path)?;
    let gate = &card.a37_diversity;
    let admission = TemplateA37Admission {
        schema_version: A37_ADMISSION_VERSION,
        source: "assay_ensemble_card_a37".to_string(),
        gate_eligible: gate.status == A37_DIVERSITY_GATE_PASSED,
        status: gate.status.clone(),
        verdict: gate.verdict.clone(),
        content_lens_count: gate.content_lens_count,
        temporal_sidecar_count: gate.temporal_sidecar_count,
        temporal_counts_toward_content_floor: gate.temporal_counts_toward_content_floor,
        association_family_count: gate.association_family_count,
        n_eff: Some(gate.n_eff),
        mean_pairwise_corr: Some(gate.mean_pairwise_corr),
        mean_pairwise_nmi: Some(gate.mean_pairwise_nmi),
        sum_unique_pid_bits: Some(gate.sum_unique_pid_bits),
    };
    let card_ref = TemplateA37CardRef {
        path: path.display().to_string(),
        blake3_hex: hash,
        card_schema_version: card.schema_version,
        card_source: card.source.clone(),
        panel_lens_count: card.panel_lens_count,
        status: gate.status.clone(),
    };
    Ok((admission, Some(card_ref)))
}

fn a37_admission_from_multi_anchor_card(
    template: &SavedPanelTemplate,
    path: &Path,
) -> CliResult<(TemplateA37Admission, TemplateA37CardRef)> {
    let bytes = fs::read(path)?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let card = serde_json::from_slice::<MultiAnchorAdmissionCard>(&bytes).map_err(|error| {
        template_error(
            TEMPLATE_INVALID,
            format!(
                "A37 admission card {} is not a multi-anchor admission card: {error}",
                path.display()
            ),
            "pass the JSON produced by calyx assay multi-anchor-card",
        )
    })?;
    ensure_multi_anchor_card_matches_template(template, &card, path)?;
    let gate_eligible = card.gate_passed && card.status == A37_DIVERSITY_GATE_PASSED;
    let admission = TemplateA37Admission {
        schema_version: A37_ADMISSION_VERSION,
        source: "assay_multi_anchor_a37_admission".to_string(),
        gate_eligible,
        status: card.status.clone(),
        verdict: format!(
            "A37 multi-anchor status={}; family_span={}; redundancy_bound={}; no_collapse={}",
            card.status, card.family_span_pass, card.redundancy_bound_pass, card.no_collapse_pass
        ),
        content_lens_count: card.lens_count,
        temporal_sidecar_count: template.time_controls.len(),
        temporal_counts_toward_content_floor: false,
        association_family_count: card.association_family_count,
        n_eff: None,
        mean_pairwise_corr: None,
        mean_pairwise_nmi: None,
        sum_unique_pid_bits: None,
    };
    let card_ref = TemplateA37CardRef {
        path: path.display().to_string(),
        blake3_hex: hash,
        card_schema_version: card.schema_version,
        card_source: card.role.clone(),
        panel_lens_count: card.lens_count,
        status: card.status,
    };
    Ok((admission, card_ref))
}

fn missing_a37_admission(template: &SavedPanelTemplate) -> TemplateA37Admission {
    let families = template
        .lenses
        .iter()
        .filter(|lens| lens.counts_toward_a35)
        .map(|lens| a37_association_family(&lens.lens_name))
        .filter(|family| *family != "temporal_sidecar")
        .collect::<BTreeSet<_>>();
    let temporal_lens_count = template
        .lenses
        .iter()
        .filter(|lens| {
            !lens.counts_toward_a35 || a37_association_family(&lens.lens_name) == "temporal_sidecar"
        })
        .count();
    TemplateA37Admission {
        schema_version: A37_ADMISSION_VERSION,
        source: "capability_card_rollup_missing_a37_pid".to_string(),
        gate_eligible: false,
        status: "missing_a37_ensemble_card".to_string(),
        verdict: "A37 gate not evaluated; capability-card rollup proves load and coverage only, not D2/D3/D4 diversity".to_string(),
        content_lens_count: template.content_lens_count(),
        temporal_sidecar_count: template.time_controls.len() + temporal_lens_count,
        temporal_counts_toward_content_floor: false,
        association_family_count: families.len(),
        n_eff: None,
        mean_pairwise_corr: None,
        mean_pairwise_nmi: None,
        sum_unique_pid_bits: None,
    }
}

fn ensure_a37_card_matches_template(
    template: &SavedPanelTemplate,
    card: &EnsembleCard,
    path: &Path,
) -> CliResult {
    let wanted = template
        .lenses
        .iter()
        .filter(|lens| lens.counts_toward_a35)
        .map(|lens| lens.lens_name.as_str())
        .collect::<BTreeSet<_>>();
    let measured = card
        .lenses
        .iter()
        .filter(|lens| a37_association_family(&lens.name) != "temporal_sidecar")
        .map(|lens| lens.name.as_str())
        .collect::<BTreeSet<_>>();
    let missing = wanted.difference(&measured).copied().collect::<Vec<_>>();
    let extra = measured.difference(&wanted).copied().collect::<Vec<_>>();
    if missing.is_empty()
        && extra.is_empty()
        && card.a37_diversity.content_lens_count == template.content_lens_count()
    {
        return Ok(());
    }
    Err(template_error(
        TEMPLATE_INVALID,
        format!(
            "A37 assay card {} does not match template {}; missing={:?}; extra={:?}; a37_content_lenses={}; template_content_lenses={}",
            path.display(),
            template.name,
            missing,
            extra,
            card.a37_diversity.content_lens_count,
            template.content_lens_count()
        ),
        "generate the Assay EnsembleCard from the same content-lens roster before profiling",
    ))
}

fn ensure_multi_anchor_card_matches_template(
    template: &SavedPanelTemplate,
    card: &MultiAnchorAdmissionCard,
    path: &Path,
) -> CliResult {
    let wanted = template
        .lenses
        .iter()
        .filter(|lens| lens.counts_toward_a35)
        .map(|lens| lens.lens_name.as_str())
        .collect::<BTreeSet<_>>();
    let measured = card
        .lenses
        .iter()
        .map(|lens| lens.name.as_str())
        .collect::<BTreeSet<_>>();
    let missing = wanted.difference(&measured).copied().collect::<Vec<_>>();
    let extra = measured.difference(&wanted).copied().collect::<Vec<_>>();
    if missing.is_empty() && extra.is_empty() && card.lens_count == template.content_lens_count() {
        return Ok(());
    }
    Err(template_error(
        TEMPLATE_INVALID,
        format!(
            "A37 admission card {} does not match template {}; missing={:?}; extra={:?}; admission_lenses={}; template_content_lenses={}",
            path.display(),
            template.name,
            missing,
            extra,
            card.lens_count,
            template.content_lens_count()
        ),
        "generate the multi-anchor admission card from the same content-lens roster before profiling",
    ))
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
