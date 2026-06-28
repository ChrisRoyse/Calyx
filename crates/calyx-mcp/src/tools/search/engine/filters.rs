use super::*;

pub(super) fn filtered_docs(
    docs: BTreeMap<CxId, Constellation>,
    raw_filter: Option<Value>,
) -> ToolResult<BTreeMap<CxId, Constellation>> {
    let filter = parse_filter(raw_filter)?;
    Ok(docs
        .into_iter()
        .filter(|(_, cx)| filter_matches(cx, &filter))
        .collect())
}

pub(super) fn parse_filter(raw: Option<Value>) -> ToolResult<QueryFilters> {
    let filters: QueryFilters = raw
        .map(serde_json::from_value)
        .transpose()
        .map_err(|err| ToolError::invalid_params(format!("parse filter JSON: {err}")))?
        .unwrap_or_default();
    filters.validate()?;
    Ok(filters)
}

pub(super) fn filter_matches(cx: &Constellation, filters: &QueryFilters) -> bool {
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

pub(super) fn scalar_matches(cx: &Constellation, filter: &ScalarPredicate) -> bool {
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

pub(super) fn anchor_matches(cx: &Constellation, filter: &AnchorPredicate) -> bool {
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

pub(super) fn metadata_matches(cx: &Constellation, filter: &MetadataPredicate) -> bool {
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

pub(super) fn anchor_value_matches(actual: &AnchorValue, expected: &AnchorValue) -> bool {
    actual == expected
}

pub(super) fn has_grounding(cx: &Constellation, anchor: Option<&AnchorKind>) -> bool {
    cx.anchors
        .iter()
        .any(|item| anchor.is_none_or(|kind| &item.kind == kind))
}
