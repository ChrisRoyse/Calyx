use calyx_core::{CalyxError, Lens, Modality, Result, SlotShape};

use super::{
    AlgorithmicLens, CALYX_STRUCTURED_BINDING_INVALID, StructuredBinding, structured_runtime_error,
};

impl AlgorithmicLens {
    /// Constructs a frozen algorithmic runtime from the canonical manifest kind.
    ///
    /// This is the single parser for persisted/direct runtime kinds. Callers must
    /// not maintain command-specific kind tables because doing so can admit a
    /// manifest that another execution path cannot load.
    pub fn from_runtime_kind(
        name: impl Into<String>,
        modality: Modality,
        kind: &str,
        declared_shape: SlotShape,
    ) -> Result<Self> {
        let lens = Self::from_runtime_kind_dim(name, modality, kind, shape_dim(declared_shape))?;
        if lens.shape() != declared_shape {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "algorithmic kind {kind} emits {:?}, but the frozen manifest declares \
                 {declared_shape:?}",
                lens.shape()
            )));
        }
        Ok(lens)
    }

    /// Constructs a frozen algorithmic runtime when only its primary dimension
    /// is known (for example, direct CLI import before a LensSpec exists).
    pub fn from_runtime_kind_dim(
        name: impl Into<String>,
        modality: Modality,
        kind: &str,
        requested_dim: u32,
    ) -> Result<Self> {
        let name = name.into();
        if requested_dim == 0 {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "algorithmic kind {kind} requires a non-zero dimension"
            )));
        }
        if StructuredBinding::is_runtime_kind(kind) {
            if modality != Modality::Structured {
                return Err(structured_runtime_error(
                    CALYX_STRUCTURED_BINDING_INVALID,
                    format!(
                        "structured algorithmic kind requires modality Structured, got {modality:?}"
                    ),
                ));
            }
            let binding = StructuredBinding::from_runtime_kind(kind)?;
            let lens = Self::structured(name, binding)?;
            if shape_dim(lens.shape()) != requested_dim {
                return Err(CalyxError::lens_dim_mismatch(format!(
                    "structured algorithmic kind emits {:?}, whose primary dimension is {}, but the frozen contract requests {requested_dim}",
                    lens.shape(),
                    shape_dim(lens.shape())
                )));
            }
            return Ok(lens);
        }
        let normalized = kind.trim().to_ascii_lowercase().replace('_', "-");
        let (base, encoded_dim) = parse_runtime_kind(&normalized)?;
        if encoded_dim.is_some_and(|dim| dim != requested_dim) {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "algorithmic kind {kind} encodes dimension {}, but the frozen contract requests \
                 {requested_dim}",
                encoded_dim.unwrap()
            )));
        }
        let lens = match base {
            "byte" | "byte-features" => Self::byte_features(name, modality),
            "scalar" => Self::scalar(name, modality),
            "one-hot" => Self::one_hot(name, modality, requested_dim),
            "ast-style" => Self::ast_style(name, modality),
            "sparse" | "sparse-keywords" => Self::sparse_keywords(name, modality, requested_dim),
            "sparse-keywords-raw-tf" => Self::sparse_keywords_raw_tf(name, modality, requested_dim),
            "token-hash" | "multi-hash" => Self::token_hash(name, modality, requested_dim),
            "legal-document-structure" => Self::legal_document_structure(name, modality),
            "gdelt-cameo" => Self::gdelt_cameo(name, modality),
            "gdelt-actor-geo" => Self::gdelt_actor_geo(name, modality, requested_dim),
            "gdelt-source-domain" => Self::gdelt_source_domain(name, modality, requested_dim),
            "gdelt-event-geo" => Self::gdelt_event_geo(name, modality, requested_dim),
            "gdelt-actor-pair" => Self::gdelt_actor_pair(name, modality, requested_dim),
            "gdelt-event-actor" => Self::gdelt_event_actor(name, modality, requested_dim),
            "gdelt-tone-signal" => Self::gdelt_tone_signal(name, modality, requested_dim),
            "gdelt-source-event" => Self::gdelt_source_event(name, modality, requested_dim),
            "gdelt-action-geo" => Self::gdelt_action_geo(name, modality, requested_dim),
            "gdelt-actor-country" => Self::gdelt_actor_country(name, modality, requested_dim),
            "gdelt-source-host" => Self::gdelt_source_host(name, modality, requested_dim),
            "gdelt-sqldate" => Self::gdelt_sql_date(name, modality, requested_dim),
            "gdelt-event-code" => Self::gdelt_event_code(name, modality, requested_dim),
            other => {
                return Err(algorithmic_kind_invalid(format!(
                    "unsupported algorithmic runtime kind {other}"
                )));
            }
        };
        if shape_dim(lens.shape()) != requested_dim {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "algorithmic kind {kind} has fixed dimension {}, but the frozen contract requests \
                 {requested_dim}",
                shape_dim(lens.shape())
            )));
        }
        Ok(lens)
    }
}

fn shape_dim(shape: SlotShape) -> u32 {
    match shape {
        SlotShape::Dense(dim) | SlotShape::Sparse(dim) => dim,
        SlotShape::Multi { token_dim } => token_dim,
    }
}

fn parse_runtime_kind(kind: &str) -> Result<(&str, Option<u32>)> {
    let Some((base, value)) = kind.rsplit_once(':') else {
        return Ok((kind, None));
    };
    let dim = value.parse::<u32>().map_err(|_| {
        algorithmic_kind_invalid(format!(
            "algorithmic runtime kind {kind} has a non-numeric dimension suffix"
        ))
    })?;
    if base.is_empty() || dim == 0 {
        return Err(algorithmic_kind_invalid(format!(
            "algorithmic runtime kind {kind} has an invalid dimension suffix"
        )));
    }
    Ok((base, Some(dim)))
}

fn algorithmic_kind_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_LENS_CONFIG_INVALID",
        message: message.into(),
        remediation: "fix the frozen algorithmic runtime kind and register a new LensId",
    }
}
