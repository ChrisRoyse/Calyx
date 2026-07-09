use super::cache::{ResponseCache, cached_json_response, store_and_respond};
use super::provenance::hex_hash;
use super::*;
use crate::blocking::run_blocking;

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
    let work_ctx = Arc::clone(&ctx);
    match run_blocking("measure", move || {
        measure_constellation(&work_ctx.vault, &work_ctx.state, input, now_ms()).map_err(|error| {
            tracing::error!(error = ?error, "CALYX_WEB_API_MEASURE_FAILED");
            ApiError::of(ErrorCode::Internal)
        })
    })
    .await
    {
        Ok(cx) => (StatusCode::OK, Json(cx)).into_response(),
        Err(error) => error.into_response(),
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

    let work_ctx = Arc::clone(&ctx);
    let query = req.query;
    let body = match run_blocking("search", move || {
        let outcome = search_outcome(
            &work_ctx.vault,
            &work_ctx.state,
            &work_ctx.vault_dir,
            &query,
            k,
            fusion,
            guard,
            None,
            false,
        )
        .map_err(|error| {
            tracing::error!(error = ?error, "CALYX_WEB_API_SEARCH_FAILED");
            ApiError::of(ErrorCode::Internal)
        })?;
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
        Ok(json!({
            "query": query,
            "k": k,
            "guardTau": outcome.guard_tau,
            "hits": hits,
        }))
    })
    .await
    {
        Ok(body) => body,
        Err(error) => return error.into_response(),
    };
    store_and_respond(&ctx.cache, cache_key, &body)
}

mod guard_support;
mod kernel;

use guard_support::{read_guard_profile, required_dense};
pub(super) use kernel::kernel_handler;
use kernel::slot_state_label;

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
    let high_stakes = req.high_stakes.unwrap_or(true);
    let work_ctx = Arc::clone(&ctx);
    let body = match run_blocking("guard", move || {
        let profile = match read_guard_profile(&work_ctx.vault) {
            Ok(Some(profile)) => profile,
            Ok(None) => {
                return Err(ApiError::new(
                    ErrorCode::BadRequest,
                    "no calibrated guard profile in this vault; run `calyx guard calibrate` first",
                ));
            }
            Err(detail) => {
                tracing::error!("CALYX_WEB_API_GUARD_PROFILE_FAILED: {detail}");
                return Err(ApiError::of(ErrorCode::Internal));
            }
        };
        let produced = required_dense(&work_ctx.state, &req.answer, &profile)?;
        let matched = required_dense(&work_ctx.state, &req.evidence, &profile)?;
        let verdict = ward_guard(&profile, &produced, &matched, high_stakes).map_err(|error| {
            tracing::error!(error = ?error, "CALYX_WEB_API_GUARD_FAILED");
            ApiError::of(ErrorCode::Internal)
        })?;
        let verdict_str = if verdict.overall_pass {
            "accept"
        } else {
            match verdict.action {
                Some(NoveltyAction::NewRegion) => "new-region",
                Some(NoveltyAction::Quarantine) => "quarantine",
                Some(NoveltyAction::RejectClosed) | None => "refuse",
            }
        };
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
        Ok(json!({
            "verdict": verdict_str,
            "overallPass": verdict.overall_pass,
            "provisional": verdict.provisional,
            "highStakes": high_stakes,
            "far": far,
            "farByAspect": far_by_aspect,
            "perSlot": per_slot,
        }))
    })
    .await
    {
        Ok(body) => body,
        Err(error) => return error.into_response(),
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// The recall gate for the website kernel (calyxdocs/12: kernel must recall the
/// corpus at >= 0.95).
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
    let work_ctx = Arc::clone(&ctx);
    let body = match run_blocking("assay_bits", move || assay_bits_body(&work_ctx)).await {
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
