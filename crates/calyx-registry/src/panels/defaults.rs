use calyx_core::{Asymmetry, Modality, SlotId, SlotShape};

use super::{AlgorithmicPanelLens, PanelLensRuntime, PanelSlotSpec, PanelTemplate};

const TEI_GTE: &str = "http://127.0.0.1:8088";

pub fn text_default() -> PanelTemplate {
    let mut slots = vec![
        tei("E1_semantic", SlotShape::Dense(768), Modality::Text),
        alg(
            "keyword_splade",
            AlgorithmicPanelLens::ByteFeatures,
            SlotShape::Sparse(30_522),
            Modality::Text,
        ),
        tei("paraphrase", SlotShape::Dense(768), Modality::Text),
        tei("entity", SlotShape::Dense(768), Modality::Text),
        tei("causal_dual", SlotShape::Dense(768), Modality::Text).with_asymmetry(Asymmetry::Dual {
            a: SlotId::new(4),
            b: SlotId::new(4),
        }),
    ];
    append_temporal(&mut slots);
    PanelTemplate {
        name: "text-default".to_string(),
        slots,
    }
}

pub fn code_default() -> PanelTemplate {
    let mut slots = [
        "semantic",
        "ast",
        "cfg",
        "dataflow",
        "type_graph",
        "trace",
        "diff",
        "oracle_anchor",
        "static_analysis",
        "runtime",
        "reasoning",
        "scalars",
    ]
    .into_iter()
    .map(|name| {
        alg(
            name,
            AlgorithmicPanelLens::ByteFeatures,
            SlotShape::Dense(16),
            Modality::Code,
        )
    })
    .collect::<Vec<_>>();
    append_temporal(&mut slots);
    PanelTemplate {
        name: "code-default".to_string(),
        slots,
    }
}

pub fn civic_default() -> PanelTemplate {
    let mut slots = (1..=21)
        .map(|idx| {
            alg(
                format!("polis_axis_{idx:02}"),
                AlgorithmicPanelLens::Scalar,
                SlotShape::Dense(1),
                Modality::Text,
            )
        })
        .collect::<Vec<_>>();
    append_temporal(&mut slots);
    PanelTemplate {
        name: "civic-default".to_string(),
        slots,
    }
}

pub fn media_default() -> PanelTemplate {
    let mut slots = vec![
        tei("semantic", SlotShape::Dense(768), Modality::Mixed),
        external("image_clip", SlotShape::Dense(512), Modality::Image),
        external("audio_wave", SlotShape::Dense(256), Modality::Audio),
        external("audio_emotion", SlotShape::Dense(128), Modality::Audio),
        external("speaker_wavlm", SlotShape::Dense(768), Modality::Audio),
        tei("transcript", SlotShape::Dense(768), Modality::Text),
        external("style_register", SlotShape::Dense(256), Modality::Mixed),
    ];
    append_temporal(&mut slots);
    PanelTemplate {
        name: "media-default".to_string(),
        slots,
    }
}

fn append_temporal(slots: &mut Vec<PanelSlotSpec>) {
    slots.push(PanelSlotSpec::temporal(
        "E2_recency",
        AlgorithmicPanelLens::TemporalRecent,
        SlotShape::Dense(1),
    ));
    slots.push(PanelSlotSpec::temporal(
        "E3_periodic",
        AlgorithmicPanelLens::TemporalPeriodic,
        SlotShape::Dense(2),
    ));
    slots.push(PanelSlotSpec::temporal(
        "E4_positional",
        AlgorithmicPanelLens::TemporalPositional,
        SlotShape::Dense(4),
    ));
}

fn tei(name: impl Into<String>, output: SlotShape, modality: Modality) -> PanelSlotSpec {
    PanelSlotSpec::content(
        name,
        PanelLensRuntime::TeiHttp {
            endpoint: TEI_GTE.to_string(),
        },
        output,
        modality,
    )
}

fn alg(
    name: impl Into<String>,
    lens: AlgorithmicPanelLens,
    output: SlotShape,
    modality: Modality,
) -> PanelSlotSpec {
    PanelSlotSpec::content(
        name,
        PanelLensRuntime::Algorithmic { lens },
        output,
        modality,
    )
}

fn external(name: impl Into<String>, output: SlotShape, modality: Modality) -> PanelSlotSpec {
    let name = name.into();
    PanelSlotSpec::content(
        name.clone(),
        PanelLensRuntime::ExternalCmd { name },
        output,
        modality,
    )
}
