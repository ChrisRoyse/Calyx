use super::cache::{cached_json_response, store_and_respond};
use super::*;
use crate::blocking::run_blocking;

// ---------------------------------------------------------------------------
// /v1/provenance/{id} — real Ledger answer-trace (#577)
// ---------------------------------------------------------------------------

/// Real vault-manifest-backed quarantine: a ledger seq is quarantined iff the
/// vault manifest says so (mirrors the CLI `calyx provenance` path). Never
/// silently treats a quarantined entry as trusted.
struct VaultQuarantine {
    vault_dir: PathBuf,
}

impl QuarantineLookup for VaultQuarantine {
    fn contains_quarantined(&self, range: std::ops::Range<u64>) -> CalyxResult<bool> {
        for seq in range {
            if is_vault_seq_quarantined(&self.vault_dir, seq)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// The vault's OWN append-only Ledger CF (via [`AsterLedgerCfStore`]) + its
/// manifest quarantine, opened once at startup. Unifies the origin: provenance
/// reads the SAME vault as measure/search/guard/kernel — no separate ledger
/// directory. Read-only by construction (this service never appends).
pub struct ProvenanceCtx {
    pub(super) store: AsterLedgerCfStore,
    quarantine: VaultQuarantine,
    /// Bounded TTL cache for `/v1/provenance/{id}` (#1898) — the headline win,
    /// since each miss does a full ledger `scan()` + `verify_chain()`.
    pub(super) cache: ResponseCache,
}

impl ProvenanceCtx {
    /// Open the vault's Ledger CF at `vault_dir`. Fails loud if the vault holds
    /// no real Aster ledger state — the service never serves provenance over an
    /// unreadable ledger.
    pub fn open(vault_dir: &FsPath) -> Result<Self, String> {
        let store = AsterLedgerCfStore::open(vault_dir)
            .map_err(|error| format!("open vault ledger {}: {error:?}", vault_dir.display()))?;
        // Fail-loud startup probe: an unscannable ledger is a hard error now.
        store
            .scan()
            .map_err(|error| format!("scan vault ledger {}: {error:?}", vault_dir.display()))?;
        Ok(Self {
            store,
            quarantine: VaultQuarantine {
                vault_dir: vault_dir.to_path_buf(),
            },
            cache: ResponseCache::from_env()?,
        })
    }

    /// Load from the required `CALYX_WEB_API_VAULT_DIR` env var (the SAME vault
    /// as measure). Fail loud if unset (no default, no fallback).
    pub fn from_env() -> Result<Self, String> {
        let dir = std::env::var("CALYX_WEB_API_VAULT_DIR").map_err(|_| {
            "CALYX_WEB_API_VAULT_DIR is required (absolute path to the vault directory)".to_string()
        })?;
        Self::open(PathBuf::from(dir).as_path())
    }
}

/// Lower-hex encode a fixed hash (BLAKE3 chain hashes are surfaced as hex).
pub(super) fn hex_hash(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Serialize one ledger entry to the #577 wire shape
/// `{seq,kind,subject,prevHash,entryHash,payload}`. The payload is decoded back
/// to JSON for the caller; an undecodable payload surfaces as `null` (the entry
/// hashes still prove what bytes were committed).
fn entry_json(entry: &LedgerEntry) -> Value {
    json!({
        "seq": entry.seq,
        "kind": serde_json::to_value(entry.kind).unwrap_or(Value::Null),
        "subject": serde_json::to_value(&entry.subject).unwrap_or(Value::Null),
        "prevHash": hex_hash(&entry.prev_hash),
        "entryHash": hex_hash(&entry.entry_hash),
        "payload": serde_json::from_slice::<Value>(&entry.payload).unwrap_or(Value::Null),
    })
}

/// Surface the real `verify_chain` verdict (Intact / Broken / Corrupt).
fn chain_json(result: &VerifyResult) -> Value {
    match result {
        VerifyResult::Intact { count } => json!({ "result": "intact", "count": count }),
        VerifyResult::Broken { at_seq, .. } => json!({ "result": "broken", "atSeq": at_seq }),
        VerifyResult::Corrupt { at_seq, reason } => {
            json!({ "result": "corrupt", "atSeq": at_seq, "reason": reason })
        }
    }
}

/// `GET /v1/provenance/{id}` — the real Ledger answer-trace for an answer id.
///
/// The `{id}` path segment is the answer id (matched against the `Query`
/// subject bytes of `Answer` ledger entries). Returns the answer trace's
/// constituent entries (answer + linked kernel + guard) in the #577 shape, the
/// `verify_chain` verdict over the whole ledger, and a `trusted` bool that is
/// true ONLY when the answer trace is itself trusted (complete + no warnings,
/// mirroring `AnswerTrace::is_trusted`) AND the hash chain verifies Intact. An
/// unknown id returns a structured `found:false` body (200), never a 500.
pub(super) async fn provenance_wired(
    State(ctx): State<Arc<ProvenanceCtx>>,
    Path(id): Path<String>,
) -> Response {
    // Serve a fresh cache hit byte-for-byte rather than re-scanning the whole
    // ledger + re-verifying the chain (#1898). Keyed by the answer id; the TTL
    // bounds staleness against an out-of-band ledger append.
    let cache_key = format!("provenance\u{1f}{id}");
    if let Some((body, age)) = ctx.cache.get(&cache_key) {
        return cached_json_response(body, "HIT", age);
    }

    let work_ctx = Arc::clone(&ctx);
    let body = match run_blocking("provenance", move || provenance_body(&work_ctx, id)).await {
        Ok(body) => body,
        Err(error) => return error.into_response(),
    };
    store_and_respond(&ctx.cache, cache_key, &body)
}

fn provenance_body(ctx: &ProvenanceCtx, id: String) -> Result<Value, ApiError> {
    // Source-of-truth scan: every read is straight off the on-disk ledger.
    let row_count = match ctx.store.scan() {
        Ok(rows) => rows.len() as u64,
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_PROVENANCE_SCAN_FAILED");
            return Err(ApiError::of(ErrorCode::Internal));
        }
    };
    let chain = match verify_chain(&ctx.store, 0..row_count) {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_PROVENANCE_VERIFY_FAILED");
            return Err(ApiError::of(ErrorCode::Internal));
        }
    };
    let trace = match get_answer_trace(&ctx.store, &ctx.quarantine, id.as_bytes()) {
        Ok(trace) => trace,
        Err(error) => {
            tracing::error!(error = ?error, "CALYX_WEB_API_PROVENANCE_TRACE_FAILED");
            return Err(ApiError::of(ErrorCode::Internal));
        }
    };

    let mut entries: Vec<Value> = [
        trace.answer_entry.as_ref(),
        trace.kernel_entry.as_ref(),
        trace.guard_entry.as_ref(),
    ]
    .into_iter()
    .flatten()
    .map(entry_json)
    .collect();
    entries.sort_by_key(|value| value["seq"].as_u64().unwrap_or(0));

    let chain_intact = matches!(chain, VerifyResult::Intact { .. });
    Ok(json!({
        "id": id,
        "found": trace.answer_entry.is_some(),
        "trusted": trace.is_trusted() && chain_intact,
        "complete": trace.complete,
        "warnings": trace.warnings.iter().map(|warning| format!("{warning:?}")).collect::<Vec<_>>(),
        "chain": chain_json(&chain),
        "entries": entries,
    }))
}
