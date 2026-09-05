use std::sync::Arc;

use calyx_core::{Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};

use crate::frozen::FrozenLensContract;
use crate::lens::ensure_input_modality;

use super::contract::structured_contract;
use super::cpu::{
    ast_style_features, byte_features, one_hot_features, scalar_features, sparse_keywords,
    sparse_keywords_raw_tf, token_hash,
};
use super::{
    AlgorithmicBatchStats, AlgorithmicEncoder, StructuredBinding, algorithmic_contract, batch,
    gdelt, legal_document,
};

#[derive(Clone, Debug)]
enum AlgorithmicRuntime {
    Builtin(AlgorithmicEncoder),
    Structured(Arc<StructuredBinding>),
}

/// A frozen algorithmic lens.
#[derive(Clone, Debug)]
pub struct AlgorithmicLens {
    id: LensId,
    modality: Modality,
    runtime: AlgorithmicRuntime,
    contract: FrozenLensContract,
    pub(super) batch: Arc<batch::BatchState>,
}

impl AlgorithmicLens {
    /// Creates an algorithmic byte-feature lens.
    pub fn byte_features(name: impl Into<String>, modality: Modality) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::ByteFeatures)
    }

    pub fn scalar(name: impl Into<String>, modality: Modality) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::Scalar)
    }

    pub fn one_hot(name: impl Into<String>, modality: Modality, buckets: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::OneHot { buckets })
    }

    pub fn ast_style(name: impl Into<String>, modality: Modality) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::AstStyle)
    }

    pub fn sparse_keywords(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::SparseKeywords { dim })
    }

    pub fn sparse_keywords_raw_tf(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(
            name,
            modality,
            AlgorithmicEncoder::SparseKeywordsRawTf { dim },
        )
    }

    pub fn token_hash(name: impl Into<String>, modality: Modality, token_dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::TokenHash { token_dim })
    }

    pub fn legal_document_structure(name: impl Into<String>, modality: Modality) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::LegalDocumentStructure)
    }

    pub fn gdelt_cameo(name: impl Into<String>, modality: Modality) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::GdeltCameo)
    }

    pub fn gdelt_actor_geo(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::GdeltActorGeo { dim })
    }

    pub fn gdelt_source_domain(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(
            name,
            modality,
            AlgorithmicEncoder::GdeltSourceDomain { dim },
        )
    }

    pub fn gdelt_event_geo(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::GdeltEventGeo { dim })
    }

    pub fn gdelt_actor_pair(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::GdeltActorPair { dim })
    }

    pub fn gdelt_event_actor(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::GdeltEventActor { dim })
    }

    pub fn gdelt_tone_signal(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::GdeltToneSignal { dim })
    }

    pub fn gdelt_source_event(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::GdeltSourceEvent { dim })
    }

    pub fn gdelt_action_geo(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::GdeltActionGeo { dim })
    }

    pub fn gdelt_actor_country(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(
            name,
            modality,
            AlgorithmicEncoder::GdeltActorCountry { dim },
        )
    }

    pub fn gdelt_source_host(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::GdeltSourceHost { dim })
    }

    pub fn gdelt_sql_date(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::GdeltSqlDate { dim })
    }

    pub fn gdelt_event_code(name: impl Into<String>, modality: Modality, dim: u32) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::GdeltEventCode { dim })
    }

    /// Creates an algorithmic lens from an encoder.
    pub fn new(name: impl Into<String>, modality: Modality, encoder: AlgorithmicEncoder) -> Self {
        let name = name.into();
        let contract = algorithmic_contract(&name, modality, encoder);
        let id = contract.lens_id();
        Self {
            id,
            modality,
            runtime: AlgorithmicRuntime::Builtin(encoder),
            contract,
            batch: Arc::new(batch::BatchState::default()),
        }
    }

    /// Creates one frozen structured lens from its canonical binding.
    pub fn structured(name: impl Into<String>, binding: StructuredBinding) -> Result<Self> {
        let name = name.into();
        let contract = structured_contract(&name, &binding)?;
        let id = contract.lens_id();
        Ok(Self {
            id,
            modality: Modality::Structured,
            runtime: AlgorithmicRuntime::Structured(Arc::new(binding)),
            contract,
            batch: Arc::new(batch::BatchState::default()),
        })
    }

    /// Returns the frozen contract that produced this lens id.
    pub fn contract(&self) -> &FrozenLensContract {
        &self.contract
    }

    /// Returns the most recent serializable batch provider/transfer evidence.
    pub fn last_batch_stats(&self) -> Option<AlgorithmicBatchStats> {
        self.batch.last_stats()
    }

    pub(super) fn builtin_encoder(&self) -> Option<AlgorithmicEncoder> {
        match &self.runtime {
            AlgorithmicRuntime::Builtin(encoder) => Some(*encoder),
            AlgorithmicRuntime::Structured(_) => None,
        }
    }

    pub(super) fn is_structured(&self) -> bool {
        matches!(&self.runtime, AlgorithmicRuntime::Structured(_))
    }

    pub(super) fn stats_label(&self) -> String {
        match &self.runtime {
            AlgorithmicRuntime::Builtin(encoder) => format!("{encoder:?}"),
            AlgorithmicRuntime::Structured(binding) => {
                format!("StructuredV1({:?})", binding.selector())
            }
        }
    }

    pub(super) fn measure_cpu(&self, input: &Input) -> Result<SlotVector> {
        let encoder = match &self.runtime {
            AlgorithmicRuntime::Builtin(encoder) => *encoder,
            AlgorithmicRuntime::Structured(binding) => return binding.measure(input),
        };
        Ok(match encoder {
            AlgorithmicEncoder::ByteFeatures => SlotVector::Dense {
                dim: encoder.dim(),
                data: byte_features(&input.bytes),
            },
            AlgorithmicEncoder::Scalar => SlotVector::Dense {
                dim: encoder.dim(),
                data: scalar_features(&input.bytes),
            },
            AlgorithmicEncoder::OneHot { buckets } => SlotVector::Dense {
                dim: encoder.dim(),
                data: one_hot_features(&input.bytes, buckets),
            },
            AlgorithmicEncoder::AstStyle => SlotVector::Dense {
                dim: encoder.dim(),
                data: ast_style_features(&input.bytes),
            },
            AlgorithmicEncoder::SparseKeywords { dim } => sparse_keywords(&input.bytes, dim)?,
            AlgorithmicEncoder::SparseKeywordsRawTf { dim } => {
                sparse_keywords_raw_tf(&input.bytes, dim)?
            }
            AlgorithmicEncoder::TokenHash { token_dim } => token_hash(&input.bytes, token_dim)?,
            AlgorithmicEncoder::LegalDocumentStructure => SlotVector::Dense {
                dim: encoder.dim(),
                data: legal_document::features(&input.bytes)?,
            },
            AlgorithmicEncoder::GdeltCameo => SlotVector::Dense {
                dim: encoder.dim(),
                data: gdelt::cameo_features(&input.bytes),
            },
            AlgorithmicEncoder::GdeltActorGeo { dim } => gdelt::actor_geo(&input.bytes, dim)?,
            AlgorithmicEncoder::GdeltSourceDomain { dim } => {
                gdelt::source_domain(&input.bytes, dim)?
            }
            AlgorithmicEncoder::GdeltEventGeo { dim } => gdelt::event_geo(&input.bytes, dim)?,
            AlgorithmicEncoder::GdeltActorPair { dim } => gdelt::actor_pair(&input.bytes, dim)?,
            AlgorithmicEncoder::GdeltEventActor { dim } => gdelt::event_actor(&input.bytes, dim)?,
            AlgorithmicEncoder::GdeltToneSignal { dim } => gdelt::tone_signal(&input.bytes, dim)?,
            AlgorithmicEncoder::GdeltSourceEvent { dim } => gdelt::source_event(&input.bytes, dim)?,
            AlgorithmicEncoder::GdeltActionGeo { dim } => gdelt::action_geo(&input.bytes, dim)?,
            AlgorithmicEncoder::GdeltActorCountry { dim } => {
                gdelt::actor_country(&input.bytes, dim)?
            }
            AlgorithmicEncoder::GdeltSourceHost { dim } => {
                gdelt::source_host_lens(&input.bytes, dim)?
            }
            AlgorithmicEncoder::GdeltSqlDate { dim } => gdelt::sql_date(&input.bytes, dim)?,
            AlgorithmicEncoder::GdeltEventCode { dim } => gdelt::event_code(&input.bytes, dim)?,
        })
    }
}

impl Lens for AlgorithmicLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        self.contract.shape()
    }

    fn modality(&self) -> Modality {
        self.modality
    }

    fn accepts_input(&self, input: &Input) -> bool {
        match &self.runtime {
            AlgorithmicRuntime::Builtin(_) => input.supports_modality(self.modality),
            AlgorithmicRuntime::Structured(_) => input.structured().is_some(),
        }
    }

    fn requires_structured_record(&self) -> bool {
        self.is_structured()
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        ensure_input_modality(self, input)?;
        let output = self.measure_cpu(input)?;
        self.batch.record(batch::cpu_stats_label(
            self.stats_label(),
            std::slice::from_ref(input),
            1,
            "single-input CPU path",
        ));
        Ok(output)
    }

    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        batch::measure_batch(self, inputs)
    }
}
