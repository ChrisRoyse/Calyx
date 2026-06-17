#[cfg(test)]
use calyx_core::{AnchorValue, Constellation};
use calyx_sextant::QueryFilters;
#[cfg(test)]
use calyx_sextant::{AnchorPredicate, MetadataPredicate, ScalarOp, ScalarPredicate};

use crate::error::{CliError, CliResult};

pub(super) fn parse(raw: Option<&str>) -> CliResult<QueryFilters> {
    let filters = match raw {
        Some(value) => serde_json::from_str::<QueryFilters>(value)
            .map_err(|err| CliError::usage(format!("parse --filter JSON: {err}")))?,
        None => QueryFilters::default(),
    };
    filters.validate()?;
    Ok(filters)
}

#[cfg(test)]
pub(super) fn matches(cx: &Constellation, filters: &QueryFilters) -> bool {
    filters
        .scalars
        .iter()
        .all(|filter| scalar_matches(cx, filter))
        && filters
            .anchors
            .iter()
            .all(|filter| anchor_matches(cx, filter))
        && filters
            .metadata
            .iter()
            .all(|filter| metadata_matches(cx, filter))
}

#[cfg(test)]
fn scalar_matches(cx: &Constellation, filter: &ScalarPredicate) -> bool {
    cx.scalars
        .get(&filter.name)
        .is_some_and(|actual| match filter.op {
            ScalarOp::Eq => actual == &filter.value,
            ScalarOp::Gt => *actual > filter.value,
            ScalarOp::Gte => *actual >= filter.value,
            ScalarOp::Lt => *actual < filter.value,
            ScalarOp::Lte => *actual <= filter.value,
        })
}

#[cfg(test)]
fn anchor_matches(cx: &Constellation, filter: &AnchorPredicate) -> bool {
    cx.anchors.iter().any(|anchor| {
        anchor.kind == filter.kind
            && filter
                .value
                .as_ref()
                .is_none_or(|value| anchor_value_matches(&anchor.value, value))
            && filter
                .min_confidence
                .is_none_or(|minimum| anchor.confidence >= minimum)
            && filter
                .source
                .as_ref()
                .is_none_or(|source| &anchor.source == source)
    })
}

#[cfg(test)]
fn anchor_value_matches(actual: &AnchorValue, expected: &AnchorValue) -> bool {
    actual == expected
}

#[cfg(test)]
fn metadata_matches(cx: &Constellation, filter: &MetadataPredicate) -> bool {
    match filter {
        MetadataPredicate::Vault(vault) => cx.vault_id == *vault,
        MetadataPredicate::Modality(modality) => cx.modality == *modality,
        MetadataPredicate::PanelVersion(version) => cx.panel_version == *version,
        MetadataPredicate::CreatedAt { min, max } => {
            min.is_none_or(|value| cx.created_at >= value)
                && max.is_none_or(|value| cx.created_at <= value)
        }
        MetadataPredicate::InputRedacted(expected) => cx.input_ref.redacted == *expected,
        MetadataPredicate::InputPointerContains(fragment) => cx
            .input_ref
            .pointer
            .as_deref()
            .is_some_and(|pointer| pointer.contains(fragment)),
    }
}
