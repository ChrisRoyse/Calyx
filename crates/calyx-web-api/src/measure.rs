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
        let state = load_vault_panel_state(vault_dir)
            .map_err(|error| format!("load panel state {}: {error:?}", vault_dir.display()))?;
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

    // The content slot is the active dense text lens (probe-measured so we don't
    // guess); without one there is nothing to embed a kernel over.
    let content_slot = match measure_query_vectors(&ctx.state, "calyx") {
        Ok(measured) => match measured
            .iter()
            .find_map(|(slot, vector)| vector.as_dense().map(|_| *slot))
        {
            Some(slot) => slot,
            None => {
                return ApiError::new(
                    ErrorCode::BadRequest,
                    "vault has no active dense text lens to build a kernel over",
                )
                .into_response();
            }
        },
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_KERNEL_PROBE_FAILED");
            return ApiError::of(ErrorCode::Internal).into_response();
        }
    };
    let kernel_params = KernelParams::default();
    let recall_params = RecallTestParams {
        min_recall_ratio: KERNEL_RECALL_GATE,
        ..RecallTestParams::default()
    };
    let (measured, contributions) = match measured_kernel_with_contributions_from_vault(
        &ctx.vault,
        content_slot,
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
    let body = json!({
        "kernelId": measured.kernel.kernel_id.to_string(),
        "panelVersion": measured.kernel.panel_version,
        "recallGate": KERNEL_RECALL_GATE,
        "members": members,
        "kernelSize": measured.kernel.members.len(),
        "corpusSize": measured.corpus_size,
        "groundedFraction": measured.kernel.groundedness.reached_anchor,
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}
