use super::cache::{ResponseCache, cached_json_response, store_and_respond};
use super::provenance::hex_hash;
use super::*;

/// Vault + panel loaded once at startup, shared read-only across requests, used
/// by the wired `/v1/measure` endpoint.
pub struct MeasureCtx {
    vault: AsterVault,
    pub(super) state: VaultPanelState,
    /// The vault directory — needed by `/v1/search` to open the persisted
    /// search indexes (`idx/search/*`) under it.
    vault_dir: PathBuf,
    /// Bounded TTL cache for the idempotent `/v1/search` results (#1898).
    pub(super) cache: ResponseCache,
}

impl MeasureCtx {
    /// Open the vault at `vault_dir` (whose final path component is the vault
    /// id) using the CLI-compatible salt `calyx-cli-vault:{id}:{name}` and load
    /// its panel. Fails loud at every step — there is no default or fallback.
    pub fn load(vault_dir: &FsPath, name: &str) -> Result<Self, String> {
        let vault_id: VaultId = vault_dir
            .file_name()
            .and_then(|component| component.to_str())
            .ok_or_else(|| format!("vault dir has no final component: {}", vault_dir.display()))?
            .parse()
            .map_err(|error| {
                format!(
                    "vault dir name is not a vault id ({}): {error}",
                    vault_dir.display()
                )
            })?;
        let salt = format!("calyx-cli-vault:{vault_id}:{name}").into_bytes();
        let vault = AsterVault::open(vault_dir, vault_id, salt, VaultOptions::default())
            .map_err(|error| format!("open vault {}: {error:?}", vault_dir.display()))?;
        let state = load_vault_panel_state(vault_dir).map_err(|error| {
            format!("load vault panel state {}: {error:?}", vault_dir.display())
        })?;
        Ok(Self {
            vault,
            state,
            vault_dir: vault_dir.to_path_buf(),
            cache: ResponseCache::from_env()?,
        })
    }

    /// Load from the required `CALYX_WEB_API_VAULT_DIR` + `CALYX_WEB_API_VAULT_NAME`
    /// env vars. Fail loud if either is unset.
    pub fn from_env() -> Result<Self, String> {
        let dir = std::env::var("CALYX_WEB_API_VAULT_DIR").map_err(|_| {
            "CALYX_WEB_API_VAULT_DIR is required (absolute path to the vault directory)".to_string()
        })?;
        let name = std::env::var("CALYX_WEB_API_VAULT_NAME").map_err(|_| {
            "CALYX_WEB_API_VAULT_NAME is required (vault name used at creation, for the salt)"
                .to_string()
        })?;
        Self::load(PathBuf::from(dir).as_path(), &name)
    }
}

/// Request body for `POST /v1/measure`.
#[derive(Deserialize)]
pub(super) struct MeasureReq {
    text: String,
}

/// Measure the input text through the loaded vault panel and return the full
/// per-lens constellation (no-flatten). Byte-identical to the CLI `calyx
/// measure` for the same input (minus the call-time `created_at`/provenance).
/// A lens-runtime failure is logged in full and returned as a generic 500 (the
/// caller envelope never carries engine internals).
pub(super) async fn measure(
    State(ctx): State<Arc<MeasureCtx>>,
    Json(req): Json<MeasureReq>,
) -> Response {
    let input = Input::new(Modality::Text, req.text.into_bytes());
    match measure_constellation(&ctx.vault, &ctx.state, input, now_ms()) {
        Ok(cx) => (StatusCode::OK, Json(cx)).into_response(),
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_MEASURE_FAILED");
            ApiError::of(ErrorCode::Internal).into_response()
        }
    }
}

/// Request body for `POST /v1/search`. `k`/`guard`/`fusion` are optional with
/// safe defaults (10 / off / rrf); invalid values fail loud (BadRequest), never
/// silently clamp.
#[derive(Deserialize)]
pub(super) struct SearchReq {
    query: String,
    #[serde(default)]
    k: Option<usize>,
    #[serde(default)]
    guard: Option<bool>,
    #[serde(default)]
    fusion: Option<String>,
}

/// Run the real Sextant search over the loaded vault and return ranked evidence
/// with stored provenance. The ranking path is the SAME `calyx_search::
/// search_outcome` the CLI `calyx search` uses (no duplication, no mocks), so
/// HTTP results match the CLI byte-for-byte on the same vault.
pub(super) async fn search(
    State(ctx): State<Arc<MeasureCtx>>,
    Json(req): Json<SearchReq>,
) -> Response {
    let k = req.k.unwrap_or(10);
    if k == 0 {
        return ApiError::new(ErrorCode::BadRequest, "k must be greater than zero").into_response();
    }
    let (fusion, fusion_label) = match req.fusion.as_deref() {
        None | Some("rrf") => (FusionChoice::Rrf, "rrf"),
        Some("weighted-rrf") => (FusionChoice::WeightedRrf, "weighted-rrf"),
        Some("single-lens") => (FusionChoice::SingleLens, "single-lens"),
        Some("kernel-first") => (FusionChoice::KernelFirst, "kernel-first"),
        Some("pipeline") => (FusionChoice::Pipeline, "pipeline"),
        Some(other) => {
            return ApiError::new(
                ErrorCode::BadRequest,
                format!(
                    "unknown fusion '{other}' (rrf|weighted-rrf|single-lens|kernel-first|pipeline)"
                ),
            )
            .into_response();
        }
    };
    let guard_on = req.guard.unwrap_or(false);
    let guard = if guard_on {
        GuardChoice::InRegion
    } else {
        GuardChoice::Off
    };

    // Idempotent for (query,k,guard,fusion) at a fixed vault state — serve a
    // fresh cache hit byte-for-byte rather than re-running Sextant (#1898). The
    // \u{1f} (unit separator) cannot appear in the label/bool fields and so
    // keeps the composite key unambiguous across the free-text query.
    let cache_key = format!(
        "search\u{1f}{k}\u{1f}{guard_on}\u{1f}{fusion_label}\u{1f}{}",
        req.query
    );
    if let Some((body, age)) = ctx.cache.get(&cache_key) {
        return cached_json_response(body, "HIT", age);
    }

    match search_outcome(
        &ctx.vault,
        &ctx.state,
        &ctx.vault_dir,
        &req.query,
        k,
        fusion,
        guard,
        None,
        false,
    ) {
        Ok(outcome) => {
            let hits: Vec<Value> = outcome
                .hits
                .iter()
                .map(|hit| {
                    json!({
                        "rank": hit.rank,
                        "cxId": hit.cx_id.to_string(),
                        "score": hit.score,
                        "provenance": {
                            "ledgerSeq": hit.provenance.seq,
                            "chainHash": hex_hash(&hit.provenance.hash),
                        },
                    })
                })
                .collect();
            let body = json!({
                "query": req.query,
                "k": k,
                "guardTau": outcome.guard_tau,
                "hits": hits,
            });
            store_and_respond(&ctx.cache, cache_key, &body)
        }
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_SEARCH_FAILED");
            ApiError::of(ErrorCode::Internal).into_response()
        }
    }
}

/// The Guard CF key for the default calibrated profile (`profile\0default`),
/// matching the CLI `calyx guard calibrate` write. Read-only here.
const GUARD_DEFAULT_KEY: &[u8] = b"profile\0default";

/// Read the calibrated [`GuardProfile`] from the vault's Guard CF. `Ok(None)`
/// when no profile has been calibrated (caller maps to a structured error — the
/// guard is NEVER run against an uncalibrated/absent profile).
fn read_guard_profile(vault: &AsterVault) -> Result<Option<GuardProfile>, String> {
    let snapshot = vault.snapshot();
    let Some(bytes) = vault
        .read_cf_at(snapshot, ColumnFamily::Guard, GUARD_DEFAULT_KEY)
        .map_err(|error| format!("read guard CF: {error:?}"))?
    else {
        return Ok(None);
    };
    serde_json::from_slice::<GuardProfile>(&bytes)
        .map(Some)
        .map_err(|error| format!("decode guard profile: {error}"))
}

/// Measure `text` through the active text lenses and extract the dense vector for
/// every `required_slot` of the profile. Fails if any required slot is not
/// measurable (fail loud — never guard on a partial slot set).
fn required_dense(
    state: &VaultPanelState,
    text: &str,
    profile: &GuardProfile,
) -> Result<std::collections::BTreeMap<calyx_core::SlotId, Vec<f32>>, ApiError> {
    let measured = measure_query_vectors(state, text).map_err(|error| {
        tracing::error!(error = ?error, "CALYX_WEB_API_GUARD_MEASURE_FAILED");
        ApiError::of(ErrorCode::Internal)
    })?;
    let by_slot: std::collections::BTreeMap<_, _> = measured.into_iter().collect();
    let mut out = std::collections::BTreeMap::new();
    for slot in &profile.required_slots {
        let dense = by_slot
            .get(slot)
            .and_then(|vector| vector.as_dense())
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCode::BadRequest,
                    format!("input is not measurable for required guard slot {slot}"),
                )
            })?;
        out.insert(*slot, dense.to_vec());
    }
    Ok(out)
}

/// Request body for `POST /v1/guard`: an answer + its evidence, both measured
/// fresh through the panel into the profile's required slots.
#[derive(Deserialize)]
pub(super) struct GuardReq {
    answer: String,
    evidence: String,
    #[serde(default)]
    high_stakes: Option<bool>,
}

/// `POST /v1/guard` — real calibrated Ward verdict. Loads the calibrated profile
/// from the vault, measures answer + evidence into the required slots, and runs
/// `calyx_ward::guard` (per-slot cosine vs conformal tau — NO flattened average,
/// INVARIANT A3). Returns accept|new-region|quarantine|refuse + the full
/// per-slot decomposition + the conformal FAR.
pub(super) async fn guard_handler(
    State(ctx): State<Arc<MeasureCtx>>,
    Json(req): Json<GuardReq>,
) -> Response {
    if req.answer.trim().is_empty() || req.evidence.trim().is_empty() {
        return ApiError::new(
            ErrorCode::BadRequest,
            "answer and evidence must both be non-empty",
        )
        .into_response();
    }
    let profile = match read_guard_profile(&ctx.vault) {
        Ok(Some(profile)) => profile,
        Ok(None) => {
            return ApiError::new(
                ErrorCode::BadRequest,
                "no calibrated guard profile in this vault; run `calyx guard calibrate` first",
            )
            .into_response();
        }
        Err(detail) => {
            tracing::error!("CALYX_WEB_API_GUARD_PROFILE_FAILED: {detail}");
            return ApiError::of(ErrorCode::Internal).into_response();
        }
    };
    let produced = match required_dense(&ctx.state, &req.answer, &profile) {
        Ok(slots) => slots,
        Err(error) => return error.into_response(),
    };
    let matched = match required_dense(&ctx.state, &req.evidence, &profile) {
        Ok(slots) => slots,
        Err(error) => return error.into_response(),
    };
    let high_stakes = req.high_stakes.unwrap_or(true);
    let verdict = match ward_guard(&profile, &produced, &matched, high_stakes) {
        Ok(verdict) => verdict,
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_GUARD_FAILED");
            return ApiError::of(ErrorCode::Internal).into_response();
        }
    };
    let verdict_str = if verdict.overall_pass {
        "accept"
    } else {
        match verdict.action {
            Some(NoveltyAction::NewRegion) => "new-region",
            Some(NoveltyAction::Quarantine) => "quarantine",
            Some(NoveltyAction::RejectClosed) | None => "refuse",
        }
    };
    // Per-slot aspect from the persisted calibration (#1899): each calibrated
    // slot carries its SlotKind (Identity/Content/Stylistic) + conformal FAR.
    // Aspect is null for a slot the profile did not calibrate, or one calibrated
    // before slot_kind was persisted — surfaced honestly, never fabricated.
    let calib_per_slot = profile.calibration.as_ref().map(|meta| &meta.per_slot);
    let per_slot: Vec<Value> = verdict
        .per_slot
        .iter()
        .map(|slot| {
            let aspect = calib_per_slot
                .and_then(|map| map.get(&slot.slot))
                .and_then(|meta| meta.slot_kind)
                .map(|kind| kind.label());
            json!({
                "slot": slot.slot.get(),
                "cosine": slot.cos,
                "tau": slot.tau,
                "pass": slot.pass,
                "aspect": aspect,
            })
        })
        .collect();
    // Conformal FAR per aspect class — the worst-case (max) calibrated FAR bound
    // across the slots sharing an aspect.
    let mut far_by_aspect: std::collections::BTreeMap<&'static str, f32> =
        std::collections::BTreeMap::new();
    if let Some(map) = calib_per_slot {
        for meta in map.values() {
            if let Some(kind) = meta.slot_kind {
                far_by_aspect
                    .entry(kind.label())
                    .and_modify(|far| *far = far.max(meta.far))
                    .or_insert(meta.far);
            }
        }
    }
    let far = profile.calibration.as_ref().map(|meta| meta.far);
    let body = json!({
        "verdict": verdict_str,
        "overallPass": verdict.overall_pass,
        "provisional": verdict.provisional,
        "highStakes": high_stakes,
        "far": far,
        "farByAspect": far_by_aspect,
        "perSlot": per_slot,
    });
    (StatusCode::OK, Json(body)).into_response()
}

/// The recall gate for the website kernel (calyxdocs/12: kernel must recall the
/// corpus at >= 0.95).
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

fn slot_state_label(state: SlotState) -> &'static str {
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
pub(super) async fn kernel_handler(State(ctx): State<Arc<MeasureCtx>>) -> Response {
    // The kernel is idempotent for a fixed vault and its leave-one-out
    // recallContribution is O(n) recall tests (#1901), so memoize the whole
    // artifact behind the bounded TTL cache (#1898) rather than recompute it per
    // call. Constant key — `/v1/kernel` takes no parameters.
    let cache_key = "kernel".to_string();
    if let Some((body, age)) = ctx.cache.get(&cache_key) {
        return cached_json_response(body, "HIT", age);
    }

    // Pick the dense text slot with the best real vault coverage. Retired and
    // parked slots remain interpretable for historical rows, so they can be a
    // better origin-artifact substrate than a newly-active lens with sparse
    // backfill.
    let content_slot = match select_kernel_content_slot(&ctx) {
        Ok(slot) => slot,
        Err(error) => return error.into_response(),
    };
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
                return ApiError::of(ErrorCode::Internal).into_response();
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
    let body = json!({
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
    });
    store_and_respond(&ctx.cache, cache_key, &body)
}

fn slot_shape_json(shape: SlotShape) -> Value {
    match shape {
        SlotShape::Dense(dim) => json!({ "kind": "dense", "dim": dim }),
        SlotShape::Sparse(dim) => json!({ "kind": "sparse", "dim": dim }),
        SlotShape::Multi { token_dim } => json!({ "kind": "multi", "tokenDim": token_dim }),
    }
}

fn modality_label(modality: Modality) -> &'static str {
    match modality {
        Modality::Text => "text",
        Modality::Code => "code",
        Modality::Image => "image",
        Modality::Audio => "audio",
        Modality::Video => "video",
        Modality::Protein => "protein",
        Modality::Dna => "dna",
        Modality::Molecule => "molecule",
        Modality::Structured => "structured",
        Modality::Mixed => "mixed",
    }
}

fn panel_assay_bits(slot: &Slot, assay_rows_available: bool) -> Value {
    if !assay_rows_available || slot.bits_about.is_empty() {
        return Value::Null;
    }
    Value::Array(
        slot.bits_about
            .iter()
            .map(|(anchor, signal)| {
                json!({
                    "anchor": serde_json::to_value(anchor).unwrap_or(Value::Null),
                    "bits": signal.bits,
                    "ci": {
                        "low": signal.ci.low,
                        "high": signal.ci.high,
                    },
                    "n": signal.n,
                    "estimator": signal.estimator,
                    "ts": signal.ts,
                })
            })
            .collect(),
    )
}

fn assay_lens_summary(slot: &Slot, assay_rows_available: bool) -> Value {
    json!({
        "slot": slot.slot_id.get(),
        "slotKey": slot.slot_key.key(),
        "state": slot_state_label(slot.state),
        "modality": modality_label(slot.modality),
        "shape": slot_shape_json(slot.shape),
        "assayBits": panel_assay_bits(slot, assay_rows_available),
    })
}

fn assay_bits_body(ctx: &MeasureCtx) -> Result<Value, ApiError> {
    let snapshot = ctx.vault.snapshot();
    let rows = ctx
        .vault
        .scan_cf_at(snapshot, ColumnFamily::Assay)
        .map_err(|error| {
            tracing::error!(error = ?error, "CALYX_WEB_API_ASSAY_BITS_SCAN_FAILED");
            ApiError::of(ErrorCode::Internal)
        })?;
    let mut assay_rows = Vec::with_capacity(rows.len());
    for (key, value) in rows {
        let key_hex = hex_hash(&key);
        let parsed = serde_json::from_slice::<Value>(&value).map_err(|error| {
            tracing::error!(%key_hex, error = ?error, "CALYX_WEB_API_ASSAY_BITS_DECODE_FAILED");
            ApiError::of(ErrorCode::Internal)
        })?;
        assay_rows.push(json!({
            "keyHex": key_hex,
            "value": parsed,
        }));
    }

    let available = !assay_rows.is_empty();
    let lenses: Vec<Value> = ctx
        .state
        .panel
        .slots
        .iter()
        .map(|slot| assay_lens_summary(slot, available))
        .collect();
    Ok(json!({
        "schemaVersion": 1,
        "source": "origin",
        "available": available,
        "reason": if available { Value::Null } else { Value::String("no_assay_rows".to_string()) },
        "panelVersion": ctx.state.panel.version,
        "rowCount": assay_rows.len(),
        "lenses": lenses,
        "rows": assay_rows,
    }))
}

/// `GET /v1/assay/bits` — raw Assay CF readback for the website artifact cache.
///
/// If the origin vault has no Assay rows yet, return a 200 with
/// `available:false` and `reason:"no_assay_rows"` so the edge can cache the
/// real absence rather than fabricating signal bits.
pub(super) async fn assay_bits_handler(State(ctx): State<Arc<MeasureCtx>>) -> Response {
    let cache_key = "assay-bits".to_string();
    if let Some((body, age)) = ctx.cache.get(&cache_key) {
        return cached_json_response(body, "HIT", age);
    }
    let body = match assay_bits_body(&ctx) {
        Ok(body) => body,
        Err(error) => return error.into_response(),
    };
    store_and_respond(&ctx.cache, cache_key, &body)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}
