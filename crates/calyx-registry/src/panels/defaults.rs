use calyx_core::{Asymmetry, Modality, SlotId, SlotShape};

use super::{AlgorithmicPanelLens, PanelLensRuntime, PanelSlotSpec, PanelTemplate};

const TEI_GTE: &str = "http://127.0.0.1:8088";

pub fn text_default() -> PanelTemplate {
    let mut slots = vec![
        tei("E1_semantic", SlotShape::Dense(768), Modality::Text),
        alg(
            "keyword_splade",
            AlgorithmicPanelLens::SparseKeywords,
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
        let lens = if name == "ast" {
            AlgorithmicPanelLens::AstStyle
        } else {
            AlgorithmicPanelLens::ByteFeatures
        };
        let output = if name == "ast" {
            SlotShape::Dense(8)
        } else {
            SlotShape::Dense(16)
        };
        alg(name, lens, output, Modality::Code)
    })
    .collect::<Vec<_>>();
    slots.push(alg(
        "lexical_sparse",
        AlgorithmicPanelLens::SparseKeywords,
        SlotShape::Sparse(30_522),
        Modality::Code,
    ));
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

pub fn legal_default() -> PanelTemplate {
    let mut slots = vec![
        registry(
            "legal_bert_small",
            "legal-bert-small",
            SlotShape::Dense(768),
            Modality::Text,
        ),
        registry(
            "general_semantic",
            "semantic-bge-small-en-v1-5",
            SlotShape::Dense(768),
            Modality::Text,
        ),
        registry(
            "keyword_splade",
            "keyword-splade",
            SlotShape::Sparse(30_522),
            Modality::Text,
        ),
        registry("entity", "entity", SlotShape::Dense(768), Modality::Text),
        registry(
            "causal_dual",
            "causal-dual",
            SlotShape::Dense(768),
            Modality::Text,
        )
        .with_asymmetry(Asymmetry::Dual {
            a: SlotId::new(4),
            b: SlotId::new(4),
        }),
    ];
    append_temporal(&mut slots);
    PanelTemplate {
        name: "legal-default".to_string(),
        slots,
    }
}

pub fn medical_default() -> PanelTemplate {
    let mut slots = vec![
        registry(
            "biomedbert_small_embeddings",
            "biomedbert-small-embeddings",
            SlotShape::Dense(768),
            Modality::Text,
        ),
        registry(
            "general_semantic",
            "semantic-bge-small-en-v1-5",
            SlotShape::Dense(768),
            Modality::Text,
        ),
        registry(
            "medical_entity",
            "medical-entity",
            SlotShape::Dense(768),
            Modality::Text,
        ),
    ];
    append_temporal(&mut slots);
    PanelTemplate {
        name: "medical-default".to_string(),
        slots,
    }
}

pub fn bio_default() -> PanelTemplate {
    let mut slots = vec![
        registry(
            "protein_esm2",
            "protein-esm2-t6-8m-adapter",
            SlotShape::Dense(16),
            Modality::Protein,
        ),
        registry(
            "dna_dnabert2",
            "dna-dnabert2-117m-adapter",
            SlotShape::Dense(16),
            Modality::Dna,
        ),
        registry(
            "molecule_chemberta",
            "molecule-chemberta-zinc-adapter",
            SlotShape::Dense(16),
            Modality::Molecule,
        ),
        registry(
            "general_semantic",
            "semantic-bge-small-en-v1-5",
            SlotShape::Dense(768),
            Modality::Text,
        ),
    ];
    append_temporal(&mut slots);
    PanelTemplate {
        name: "bio-default".to_string(),
        slots,
    }
}

pub fn media_default() -> PanelTemplate {
    let mut slots = vec![
        registry(
            "media_semantic",
            "media-semantic",
            SlotShape::Dense(768),
            Modality::Mixed,
        ),
        registry(
            "image_siglip2",
            "image-siglip2-b16-adapter",
            SlotShape::Dense(768),
            Modality::Image,
        ),
        registry(
            "audio_clap",
            "audio-clap-htsat-adapter",
            SlotShape::Dense(512),
            Modality::Audio,
        ),
        registry(
            "audio_wave",
            "audio-wave",
            SlotShape::Dense(256),
            Modality::Audio,
        ),
        registry(
            "audio_emotion",
            "audio-emotion",
            SlotShape::Dense(128),
            Modality::Audio,
        ),
        registry(
            "speaker_wavlm",
            "speaker-wavlm",
            SlotShape::Dense(512),
            Modality::Audio,
        ),
        registry(
            "transcript",
            "transcript-semantic",
            SlotShape::Dense(768),
            Modality::Text,
        ),
        registry(
            "style_register",
            "style-register",
            SlotShape::Dense(768),
            Modality::Text,
        ),
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

fn registry(
    name: impl Into<String>,
    registry_name: impl Into<String>,
    output: SlotShape,
    modality: Modality,
) -> PanelSlotSpec {
    PanelSlotSpec::registry(name, registry_name, output, modality)
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
