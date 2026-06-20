use calyx_core::{Modality, Placement};
use serde::{Deserialize, Serialize};

use super::{AnchorGap, DeficitMap, ModalityId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversionTarget {
    pub hf_id: String,
    pub modality: ModalityId,
    pub axis: String,
    pub formats: Vec<String>,
    pub expected_bits: f64,
    pub expected_cost: ExpectedTargetCost,
    pub expected_bits_per_vram_mb: Option<f64>,
    pub expected_bits_per_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExpectedTargetCost {
    pub placement: Placement,
    pub vram_mb: f64,
    pub ram_mb: f64,
    pub ms_per_input: f64,
}

impl ExpectedTargetCost {
    const fn cpu(ram_mb: f64, ms_per_input: f64) -> Self {
        Self {
            placement: Placement::Cpu,
            vram_mb: 0.0,
            ram_mb,
            ms_per_input,
        }
    }

    const fn gpu(vram_mb: f64, ram_mb: f64, ms_per_input: f64) -> Self {
        Self {
            placement: Placement::Gpu,
            vram_mb,
            ram_mb,
            ms_per_input,
        }
    }
}

pub fn ranked_conversion_targets(deficit: &DeficitMap) -> Vec<ConversionTarget> {
    let top_gap = top_gap(deficit);
    let gap_bits = top_gap.map(|gap| gap.gap).unwrap_or(0.0).max(0.0);
    let modalities = if deficit.underrepresented_modalities.is_empty() {
        vec![Modality::Mixed]
    } else {
        deficit.underrepresented_modalities.clone()
    };
    let mut targets = Vec::new();
    for modality in modalities {
        targets.extend(targets_for_modality(modality, top_gap, gap_bits));
    }
    targets.sort_by(|left, right| {
        right
            .expected_density_rank()
            .total_cmp(&left.expected_density_rank())
            .then_with(|| right.expected_bits.total_cmp(&left.expected_bits))
            .then_with(|| left.hf_id.cmp(&right.hf_id))
            .then_with(|| left.axis.cmp(&right.axis))
    });
    targets
}

fn targets_for_modality(
    modality: ModalityId,
    top_gap: Option<&AnchorGap>,
    gap_bits: f64,
) -> Vec<ConversionTarget> {
    let axis = axis_for(modality, top_gap.map(|gap| gap.anchor_class.as_str()));
    match modality {
        Modality::Protein => vec![target(
            "facebook/esm2_t6_8M_UR50D",
            modality,
            axis,
            &["adapter"],
            gap_bits,
            1.00,
            ExpectedTargetCost::gpu(64.0, 128.0, 1.2),
        )],
        Modality::Dna => vec![target(
            "zhihan1996/DNABERT-2-117M",
            modality,
            axis,
            &["adapter"],
            gap_bits,
            1.00,
            ExpectedTargetCost::gpu(470.0, 512.0, 3.8),
        )],
        Modality::Molecule => vec![target(
            "seyonec/ChemBERTa-zinc-base-v1",
            modality,
            axis,
            &["adapter"],
            gap_bits,
            1.00,
            ExpectedTargetCost::gpu(420.0, 512.0, 3.2),
        )],
        Modality::Image => vec![target(
            "google/siglip2-base-patch16-224",
            modality,
            axis,
            &["adapter"],
            gap_bits,
            0.95,
            ExpectedTargetCost::gpu(900.0, 768.0, 5.5),
        )],
        Modality::Audio => audio_targets(modality, axis, top_gap, gap_bits),
        Modality::Text | Modality::Code | Modality::Mixed | Modality::Structured => {
            text_targets(axis, top_gap, gap_bits)
        }
        Modality::Video => vec![target(
            "google/siglip2-base-patch16-224",
            Modality::Image,
            axis,
            &["adapter"],
            gap_bits,
            0.75,
            ExpectedTargetCost::gpu(900.0, 768.0, 5.5),
        )],
    }
}

fn audio_targets(
    modality: ModalityId,
    axis: String,
    top_gap: Option<&AnchorGap>,
    gap_bits: f64,
) -> Vec<ConversionTarget> {
    vec![
        target(
            "laion/clap-htsat-unfused",
            modality,
            axis.clone(),
            &["adapter"],
            gap_bits,
            audio_weight(top_gap, "clap"),
            ExpectedTargetCost::gpu(1300.0, 1024.0, 8.0),
        ),
        target(
            "Xenova/wav2vec2-base-960h",
            modality,
            axis,
            &["onnx-int8"],
            gap_bits,
            audio_weight(top_gap, "wav2vec2"),
            ExpectedTargetCost::gpu(360.0, 384.0, 3.0),
        ),
    ]
}

fn text_targets(axis: String, top_gap: Option<&AnchorGap>, gap_bits: f64) -> Vec<ConversionTarget> {
    vec![
        target(
            "Xenova/bge-small-en-v1.5",
            Modality::Text,
            axis.clone(),
            &["onnx-int8"],
            gap_bits,
            text_weight(top_gap, "bge"),
            ExpectedTargetCost::gpu(130.0, 192.0, 1.2),
        ),
        target(
            "malteos/scincl",
            Modality::Text,
            axis.clone(),
            &["onnx-fp32"],
            gap_bits,
            text_weight(top_gap, "scincl"),
            ExpectedTargetCost::gpu(750.0, 512.0, 64.0),
        ),
        target(
            "sentence-transformers/all-MiniLM-L6-v2",
            Modality::Text,
            axis.clone(),
            &["candle-fp16"],
            gap_bits,
            text_weight(top_gap, "minilm"),
            ExpectedTargetCost::gpu(90.0, 192.0, 0.9),
        ),
        target(
            "minishlab/potion-base-8M",
            Modality::Text,
            axis,
            &["model2vec"],
            gap_bits,
            text_weight(top_gap, "potion"),
            ExpectedTargetCost::cpu(64.0, 0.08),
        ),
    ]
}

fn target(
    hf_id: &str,
    modality: ModalityId,
    axis: String,
    formats: &[&str],
    gap_bits: f64,
    weight: f64,
    expected_cost: ExpectedTargetCost,
) -> ConversionTarget {
    let expected_bits = (gap_bits * weight).max(0.0).min(gap_bits);
    ConversionTarget {
        hf_id: hf_id.to_string(),
        modality,
        axis,
        formats: formats.iter().map(|format| (*format).to_string()).collect(),
        expected_bits,
        expected_cost,
        expected_bits_per_vram_mb: density_axis(expected_bits, expected_cost.vram_mb),
        expected_bits_per_ms: expected_bits / expected_cost.ms_per_input,
    }
}

impl ConversionTarget {
    fn expected_density_rank(&self) -> f64 {
        if self.expected_cost.vram_mb <= f64::EPSILON {
            return self.expected_bits_per_ms;
        }
        self.expected_bits_per_vram_mb.unwrap_or(0.0)
    }
}

fn density_axis(bits: f64, resource: f64) -> Option<f64> {
    if resource <= f64::EPSILON {
        None
    } else {
        Some(bits / resource)
    }
}

fn axis_for(modality: ModalityId, anchor: Option<&str>) -> String {
    let anchor = anchor.unwrap_or("").to_ascii_lowercase();
    match modality {
        Modality::Protein => "protein_sequence",
        Modality::Dna => "dna_sequence",
        Modality::Molecule => "molecule_structure",
        Modality::Image => "image_semantics",
        Modality::Video => "video_frame_semantics",
        Modality::Audio if anchor.contains("speaker") => "speaker_identity",
        Modality::Audio => "audio_acoustics",
        Modality::Code => "code_semantics",
        Modality::Structured if anchor.contains("time") || anchor.contains("temporal") => {
            "temporal_structured"
        }
        Modality::Structured => "structured_outcome",
        Modality::Text | Modality::Mixed
            if anchor.contains("science") || anchor.contains("domain") =>
        {
            "scientific_text"
        }
        Modality::Text | Modality::Mixed => "semantic_text",
    }
    .to_string()
}

fn text_weight(top_gap: Option<&AnchorGap>, model: &str) -> f64 {
    let anchor = top_gap
        .map(|gap| gap.anchor_class.to_ascii_lowercase())
        .unwrap_or_default();
    match model {
        "scincl" | "scibert" if anchor.contains("science") || anchor.contains("domain") => 1.05,
        "bge" => 1.00,
        "minilm" => 0.92,
        "potion" => 0.82,
        _ => 0.90,
    }
}

fn audio_weight(top_gap: Option<&AnchorGap>, model: &str) -> f64 {
    let anchor = top_gap
        .map(|gap| gap.anchor_class.to_ascii_lowercase())
        .unwrap_or_default();
    match model {
        "wav2vec2" if anchor.contains("speaker") || anchor.contains("speech") => 1.02,
        "clap" => 0.96,
        _ => 0.82,
    }
}

fn top_gap(deficit: &DeficitMap) -> Option<&AnchorGap> {
    deficit
        .top_gaps
        .iter()
        .max_by(|left, right| left.gap.total_cmp(&right.gap))
}
