use super::*;
use crate::blocking::run_blocking;

const KERNEL_RECALL_GATE: f32 = 0.95;

#[derive(Clone, Debug)]
struct KernelContentSlotCoverage {
    slot_id: SlotId,
    slot_key: String,
    state: SlotState,
    dense_dim: u32,
    embedded: usize,
    vault_total: usize,
}

fn slot_state_rank(state: SlotState) -> u8 {
    match state {
        SlotState::Active => 0,
        SlotState::Parked => 1,
        SlotState::Retired => 2,
    }
}

pub(super) fn slot_state_label(state: SlotState) -> &'static str {
    match state {
        SlotState::Active => "active",
        SlotState::Parked => "parked",
        SlotState::Retired => "retired",
    }
}

fn dense_text_panel_slots(slots: &[Slot]) -> Vec<&Slot> {
    slots
        .iter()
        .filter(|slot| slot.modality == Modality::Text)
        .filter(|slot| matches!(slot.shape, SlotShape::Dense(_)))
        .collect()
}

fn cx_id_from_base_key(key: &[u8]) -> Result<CxId, ApiError> {
    let bytes: [u8; 16] = key.try_into().map_err(|_| {
        ApiError::new(
            ErrorCode::Internal,
            format!("base CF key has {} bytes, expected 16", key.len()),
        )
    })?;
    Ok(CxId::from_bytes(bytes))
}

fn select_kernel_content_slot(ctx: &MeasureCtx) -> Result<KernelContentSlotCoverage, ApiError> {
    let candidates = dense_text_panel_slots(&ctx.state.panel.slots);
    if candidates.is_empty() {
        return Err(ApiError::new(
            ErrorCode::BadRequest,
            "vault has no dense text lens to build a kernel over",
        ));
    }

    let mut coverage_by_slot: std::collections::BTreeMap<SlotId, usize> =
        std::collections::BTreeMap::new();
    let snapshot = ctx.vault.snapshot();
    let base_rows = ctx
        .vault
        .scan_cf_at(snapshot, ColumnFamily::Base)
        .map_err(|error| {
            tracing::error!(error = ?error, "CALYX_WEB_API_KERNEL_COVERAGE_SCAN_FAILED");
            ApiError::of(ErrorCode::Internal)
        })?;
    for (key, _) in &base_rows {
        let cx_id = cx_id_from_base_key(key)?;
        let cx = ctx.vault.get(cx_id, snapshot).map_err(|error| {
            tracing::error!(error = ?error, cx_id = %cx_id, "CALYX_WEB_API_KERNEL_COVERAGE_READ_FAILED");
            ApiError::of(ErrorCode::Internal)
        })?;
        for slot in &candidates {
            if cx
                .slots
                .get(&slot.slot_id)
                .and_then(|vector| vector.as_dense())
                .is_some()
            {
                *coverage_by_slot.entry(slot.slot_id).or_insert(0) += 1;
            }
        }
    }

    let vault_total = base_rows.len();
    let mut coverage: Vec<KernelContentSlotCoverage> = candidates
        .iter()
        .map(|slot| KernelContentSlotCoverage {
            slot_id: slot.slot_id,
            slot_key: slot.slot_key.key().to_string(),
            state: slot.state,
            dense_dim: match slot.shape {
                SlotShape::Dense(dim) => dim,
                SlotShape::Sparse(_) | SlotShape::Multi { .. } => 0,
            },
            embedded: coverage_by_slot
                .get(&slot.slot_id)
                .copied()
                .unwrap_or_default(),
            vault_total,
        })
        .collect();
    coverage.sort_by(|left, right| {
        right
            .embedded
            .cmp(&left.embedded)
            .then_with(|| slot_state_rank(left.state).cmp(&slot_state_rank(right.state)))
            .then_with(|| left.slot_id.cmp(&right.slot_id))
    });

    coverage
        .into_iter()
        .find(|slot| slot.embedded >= 2)
        .ok_or_else(|| {
            ApiError::new(
                ErrorCode::BadRequest,
                "vault has fewer than two embedded concepts across dense text slots",
            )
        })
}

/// `GET /v1/kernel` — the real doc-corpus kernel for the loaded vault, with
/// MEASURED kernel-only recall (built by `calyx_lodestar::measured_kernel_from_vault`
/// reading per-concept embeddings straight from the constellations — no mock, no
/// fabricated recall). Members carry their A2 trust (anchored/provisional);
/// recall is measured against the full corpus index at gate 0.95.
pub(crate) async fn kernel_handler(State(ctx): State<Arc<MeasureCtx>>) -> Response {
    // The kernel is idempotent for a fixed vault and its leave-one-out
    // recallContribution is O(n) recall tests (#1901), so memoize the whole
    // artifact behind the bounded TTL cache (#1898) rather than recompute it per
    // call. Constant key — `/v1/kernel` takes no parameters.
    let cache_key = "kernel".to_string();
    if let Some((body, age)) = ctx.cache.get(&cache_key) {
        return cached_json_response(body, "HIT", age);
    }

    let work_ctx = Arc::clone(&ctx);
    let body = match run_blocking("kernel", move || kernel_body(&work_ctx)).await {
        Ok(body) => body,
        Err(error) => return error.into_response(),
    };
    store_and_respond(&ctx.cache, cache_key, &body)
}

fn kernel_body(ctx: &MeasureCtx) -> Result<Value, ApiError> {
    // Pick the dense text slot with the best real vault coverage. Retired and
    // parked slots remain interpretable for historical rows, so they can be a
    // better origin-artifact substrate than a newly-active lens with sparse
    // backfill.
    let content_slot = select_kernel_content_slot(ctx)?;
    let kernel_params = KernelParams {
        panel_version: u64::from(ctx.state.panel.version),
        anchor_kind: Some("origin".to_string()),
        built_at_millis: now_ms(),
        ..KernelParams::default()
    };
    let recall_params = RecallTestParams {
        min_recall_ratio: KERNEL_RECALL_GATE,
        ..RecallTestParams::default()
    };
    let (measured, contributions) =
        match measured_kernel_with_contributions_from_vault_allow_partial(
            &ctx.vault,
            content_slot.slot_id,
            &kernel_params,
            &recall_params,
            8,
            0.5,
        ) {
            Ok(result) => result,
            Err(error) => {
                tracing::error!(error = ?error, "CALYX_WEB_API_KERNEL_FAILED");
                return Err(ApiError::of(ErrorCode::Internal));
            }
        };
    let unanchored: std::collections::BTreeSet<_> = measured
        .kernel
        .groundedness
        .unanchored_members
        .iter()
        .copied()
        .collect();
    let contribution_by_id: std::collections::BTreeMap<_, _> = contributions
        .iter()
        .map(|(id, drop)| (*id, *drop))
        .collect();
    // Concept label = the constellation's real `label:` anchor value, read from
    // the vault — null when the concept carries no label anchor (no fabrication).
    let snapshot = ctx.vault.snapshot();
    let members: Vec<Value> = measured
        .kernel
        .members
        .iter()
        .map(|cx_id| {
            let label = match ctx.vault.get(*cx_id, snapshot) {
                Ok(cx) => cx.anchors.iter().find_map(|anchor| match &anchor.kind {
                    AnchorKind::Label(value) => Some(value.clone()),
                    _ => None,
                }),
                Err(error) => {
                    tracing::error!(error = ?error, cx_id = %cx_id, "CALYX_WEB_API_KERNEL_LABEL_READ_FAILED");
                    None
                }
            };
            json!({
                "id": cx_id.to_string(),
                "trust": if unanchored.contains(cx_id) { "provisional" } else { "anchored" },
                "recallContribution": contribution_by_id.get(cx_id),
                "label": label,
            })
        })
        .collect();
    let recall = &measured.recall;
    let skipped_unembedded = measured
        .vault_corpus_size
        .saturating_sub(measured.corpus_size);
    let coverage_ratio = if content_slot.vault_total == 0 {
        0.0
    } else {
        content_slot.embedded as f64 / content_slot.vault_total as f64
    };
    Ok(json!({
        "available": true,
        "kernelId": measured.kernel.kernel_id.to_string(),
        "panelVersion": measured.kernel.panel_version,
        "recallGate": KERNEL_RECALL_GATE,
        "members": members,
        "kernelSize": measured.kernel.members.len(),
        "corpusSize": measured.corpus_size,
        "vaultCorpusSize": measured.vault_corpus_size,
        "skippedUnembedded": measured.skipped_unembedded,
        "contentSlot": content_slot.slot_id.get(),
        "contentSlotKey": content_slot.slot_key,
        "contentSlotState": slot_state_label(content_slot.state),
        "contentSlotCoverage": {
            "embedded": content_slot.embedded,
            "vaultTotal": content_slot.vault_total,
            "skippedUnembedded": skipped_unembedded,
            "ratio": coverage_ratio,
            "denseDim": content_slot.dense_dim,
        },
        "groundedFraction": measured.kernel.groundedness.reached_anchor,
        "warnings": measured.kernel.warnings,
        "recall": {
            "measured": true,
            "kernelOnly": recall.kernel_only,
            "full": recall.full,
            "ratio": recall.ratio,
            "gate": KERNEL_RECALL_GATE,
            "passed": recall.ratio >= KERNEL_RECALL_GATE,
            "nQueriesTested": recall.n_queries_tested,
            "approxFactor": recall.approx_factor,
            "warning": recall.warning,
        },
    }))
}
