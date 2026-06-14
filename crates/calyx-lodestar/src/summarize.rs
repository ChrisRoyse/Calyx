//! Universal summarization via the multi-scope kernel (PH72 · T06).
//!
//! `summarize(store, scope, …)` makes the kernel the universal summarization
//! primitive — "the core of ANY slice." For any [`Scope`] it builds (or reuses a
//! cached) kernel and returns the kernel node ids with their recall/groundedness
//! metrics. This is **structural** summarization only (strict Royse theory, A24):
//! the summary *is* the kernel nodes, never generated text.
//!
//! Each call is Ledger-provenanced (A15): a `SUMMARIZE_INVOKED` entry carrying the
//! scope hash and the result metrics is appended before the result is returned, so
//! the invocation is byte-auditable independently of the return value.
//!
//! Fail-closed boundaries (DOCTRINE §0):
//! - an inverted `TimeWindow` (`t0 > t1`) is rejected up front with
//!   [`CALYX_SCOPE_INVALID_TIME_WINDOW`] — no kernel is built;
//! - `require_grounded` with `grounded_fraction < 0.5` returns
//!   [`CALYX_SUMMARIZE_INSUFFICIENT_GROUNDING`] (no partial result);
//! - an as-of read before the retention horizon returns
//!   [`CALYX_TIMETRAVEL_BEFORE_HORIZON`] unchanged (no data).
//!
//! Note: lodestar's kernel layer is defined over the [`AssocStore`] trait. A
//! production `AsterVault → AssocStore` bridge (and a vault-embedded retention
//! horizon) does not exist yet, so `summarize_as_of` takes the historical view
//! parameters explicitly and expresses "as of `t`" by intersecting the requested
//! scope with `TimeWindow { 0, t }` over the same store. See the follow-up issue
//! for wiring a real time-travel snapshot store.

use std::fmt;

use calyx_core::{AnchorKind, CalyxError, Clock, CxId, LedgerRef, Ts};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, LedgerCfStore, SubjectId};
use serde::{Deserialize, Serialize};

use crate::error::LodestarError;
use crate::kernel::KernelParams;
use crate::kernel_graph::KernelGraphParams;
use crate::multi_scope::build_kernel;
use crate::recall_test::{AnnIndex, CorpusReader, RecallTestParams, kernel_recall_test_with_clock};
use crate::scope::{AssocStore, Scope, materialize_scope, scope_hash};
use crate::scope_cache::ScopeCache;
use crate::{EmbeddingStore, Kernel, build_kernel_index};

/// `require_grounded` rejects any kernel whose grounded fraction is below this.
const MIN_GROUNDED_FRACTION: f32 = 0.5;

/// Marker embedded in the subject and payload of every summarization Ledger entry.
pub const SUMMARIZE_INVOKED_MARKER: &str = "SUMMARIZE_INVOKED";

/// A `TimeWindow` scope had `t0 > t1`; no kernel is built (fail-closed).
pub const CALYX_SCOPE_INVALID_TIME_WINDOW: &str = "CALYX_SCOPE_INVALID_TIME_WINDOW";
/// `require_grounded` was set but the kernel grounded fraction is `< 0.5`; the
/// summary is withheld rather than returned partially grounded (fail-closed, A16).
pub const CALYX_SUMMARIZE_INSUFFICIENT_GROUNDING: &str = "CALYX_SUMMARIZE_INSUFFICIENT_GROUNDING";
/// An as-of summarization asked for a time before the retention horizon; no
/// historical data is materialized (fail-closed).
pub const CALYX_TIMETRAVEL_BEFORE_HORIZON: &str = "CALYX_TIMETRAVEL_BEFORE_HORIZON";

/// Optional knobs for [`summarize`]. All default to the "natural" behavior.
#[derive(Clone, Debug, PartialEq)]
pub struct SummarizeParams {
    /// Hard upper bound on kernel node count. `None` uses the kernel pipeline's
    /// default target fraction. `Some(n)` constrains the target fraction so the
    /// kernel holds at most ~`n` nodes (metrics are computed on that kernel).
    pub max_kernel_size: Option<usize>,
    /// When `true`, a kernel with `grounded_fraction < 0.5` is rejected.
    pub require_grounded: bool,
    /// Cache lifetime hint. `Some(0)` bypasses the shared cache (every call
    /// recomputes and writes a fresh Ledger entry); any other value reuses the
    /// caller-owned [`ScopeCache`].
    pub cache_ttl_secs: Option<u64>,
    /// Anchor kind used to ground the kernel. `None` lets the scope decide (a
    /// `Domain` scope names its own kind; other scopes ground only against
    /// anchors the scope itself carries). Supply this to ground an
    /// `AllAssociations`/`Collection`/`TimeWindow` summary against a domain.
    pub anchor_kind: Option<AnchorKind>,
}

impl Default for SummarizeParams {
    fn default() -> Self {
        Self {
            max_kernel_size: None,
            require_grounded: false,
            cache_ttl_secs: Some(3600),
            anchor_kind: None,
        }
    }
}

/// The structural summary of a scope: the kernel node ids plus their metrics and
/// the Ledger reference of the `SUMMARIZE_INVOKED` provenance entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SummarizeResult {
    /// BLAKE3 hash of the scope this summary is for.
    pub scope_hash: [u8; 32],
    /// The kernel nodes — the summary itself (a strict subset of the scope).
    pub kernel_ids: Vec<CxId>,
    /// `kernel_ids.len()`.
    pub kernel_size: usize,
    /// Kernel-only recall in `[0, 1]` as reported by the kernel pipeline. This is
    /// `0.0` until a recall pass measures it: recall measurement needs per-node
    /// embeddings + an ANN index, which the [`AssocStore`] graph (edge weights
    /// only) does not carry. Wiring measured recall is gated on the production
    /// Vault→(AssocStore + embeddings) bridge — see the T06 follow-up issue.
    pub kernel_only_recall: f32,
    /// Fraction of kernel members that reached an anchor, in `[0, 1]` — genuinely
    /// computed by the kernel grounding pass.
    pub grounded_fraction: f32,
    /// DFVS recall approximation factor from the kernel pipeline (computed).
    pub approx_factor: f32,
    /// Ledger reference of the `SUMMARIZE_INVOKED` entry for this call.
    pub ledger_ref: LedgerRef,
}

/// Optional recall inputs for a measured `kernel_only_recall`.
pub struct SummarizeRecall<'a> {
    pub embeddings: &'a dyn EmbeddingStore,
    pub full_index: &'a dyn AnnIndex,
    pub corpus: &'a dyn CorpusReader,
    pub params: RecallTestParams,
}

impl fmt::Display for SummarizeResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "scope_hash                         | kernel_size | recall | grounded_fraction | approx_factor"
        )?;
        write!(
            f,
            "{} | {:>11} | {:>6.4} | {:>17.4} | {:>13.4}",
            hex32(&self.scope_hash),
            self.kernel_size,
            self.kernel_only_recall,
            self.grounded_fraction,
            self.approx_factor,
        )
    }
}

/// The shared engine plumbing every summarization call needs: the caller-owned
/// scope cache, the clock that stamps the build and the Ledger entry, and the
/// provenance sink.
pub struct SummarizeCtx<'a, S, C>
where
    S: LedgerCfStore,
    C: Clock,
{
    /// Caller-owned scope cache (reused unless `cache_ttl_secs == Some(0)`).
    pub cache: &'a mut ScopeCache,
    /// Stamps the kernel build time and the Ledger entry.
    pub clock: &'a dyn Clock,
    /// Append-only provenance sink for the `SUMMARIZE_INVOKED` entry.
    pub ledger: &'a mut LedgerAppender<S, C>,
}

/// Builds (or reuses) the kernel for `scope` and returns it as a structural
/// summary, appending a `SUMMARIZE_INVOKED` Ledger entry (A15).
pub fn summarize<S, C>(
    store: &dyn AssocStore,
    scope: Scope,
    params: Option<SummarizeParams>,
    ctx: &mut SummarizeCtx<'_, S, C>,
) -> Result<SummarizeResult, CalyxError>
where
    S: LedgerCfStore,
    C: Clock,
{
    summarize_with_recall(store, scope, params, None, ctx)
}

/// Same as [`summarize`], but measures `kernel_only_recall` against a supplied
/// full corpus/index before writing provenance.
pub fn summarize_with_recall<S, C>(
    store: &dyn AssocStore,
    scope: Scope,
    params: Option<SummarizeParams>,
    recall: Option<SummarizeRecall<'_>>,
    ctx: &mut SummarizeCtx<'_, S, C>,
) -> Result<SummarizeResult, CalyxError>
where
    S: LedgerCfStore,
    C: Clock,
{
    summarize_with_ledger(
        store,
        scope,
        params,
        recall,
        ctx.cache,
        ctx.clock,
        |scope_h, kernel_size, kernel_only_recall, grounded_fraction| {
            append_invoked(
                ctx.ledger,
                scope_h,
                kernel_size,
                kernel_only_recall,
                grounded_fraction,
            )
        },
    )
}

/// Advanced entry point for callers that append provenance through a non-generic
/// sink such as `AsterVault::append_ledger_entry`.
pub fn summarize_with_ledger<F>(
    store: &dyn AssocStore,
    scope: Scope,
    params: Option<SummarizeParams>,
    recall: Option<SummarizeRecall<'_>>,
    cache: &mut ScopeCache,
    clock: &dyn Clock,
    mut append: F,
) -> Result<SummarizeResult, CalyxError>
where
    F: FnMut(&[u8; 32], usize, f32, f32) -> Result<LedgerRef, CalyxError>,
{
    let params = params.unwrap_or_default();
    validate_scope(&scope)?;

    let scope_h = scope_hash(&scope);
    let now = clock.now();

    let mut kernel_params = KernelParams {
        built_at_millis: now,
        ..KernelParams::default()
    };
    if let Some(max) = params.max_kernel_size {
        kernel_params.kernel_graph =
            target_fraction_for_cap(&scope, store, max, kernel_params.kernel_graph)?;
    }

    // `cache_ttl_secs == Some(0)` => no cross-call reuse: drive the build with a
    // throwaway cache so two successive calls both recompute (and re-provenance).
    let mut throwaway = ScopeCache::new(1);
    let active_cache = if params.cache_ttl_secs == Some(0) {
        &mut throwaway
    } else {
        &mut *cache
    };

    let mut kernel = build_kernel(
        store,
        scope,
        params.anchor_kind.clone(),
        kernel_params,
        active_cache,
    )
    .map_err(to_calyx)?;

    let grounded_fraction = kernel.groundedness.reached_anchor;
    if params.require_grounded && grounded_fraction < MIN_GROUNDED_FRACTION {
        return Err(insufficient_grounding(grounded_fraction));
    }

    apply_measured_recall(&mut kernel, recall.as_ref(), clock)?;

    let kernel_ids = kernel.members.clone();
    let kernel_size = kernel_ids.len();
    let kernel_only_recall = kernel.recall.kernel_only;
    let approx_factor = kernel.recall.approx_factor as f32;

    let ledger_ref = append(&scope_h, kernel_size, kernel_only_recall, grounded_fraction)?;

    Ok(SummarizeResult {
        scope_hash: scope_h,
        kernel_ids,
        kernel_size,
        kernel_only_recall,
        grounded_fraction,
        approx_factor,
        ledger_ref,
    })
}

fn apply_measured_recall(
    kernel: &mut Kernel,
    recall: Option<&SummarizeRecall<'_>>,
    clock: &dyn Clock,
) -> Result<(), CalyxError> {
    let Some(recall) = recall else {
        return Ok(());
    };
    if kernel.members.is_empty() {
        return Ok(());
    }
    let approx_factor = kernel.recall.approx_factor;
    let tau_star_estimate = kernel.recall.tau_star_estimate;
    let tau_star_exact = kernel.recall.tau_star_exact;
    let index = build_kernel_index(kernel, recall.embeddings).map_err(to_calyx)?;
    let mut measured = kernel_recall_test_with_clock(
        &index,
        recall.full_index,
        recall.corpus,
        &recall.params,
        clock,
    )
    .map_err(to_calyx)?;
    measured.approx_factor = approx_factor;
    measured.tau_star_estimate = tau_star_estimate;
    measured.tau_star_exact = tau_star_exact;
    kernel.recall = measured;
    Ok(())
}

/// Summarizes `scope` as of time `t`: the scope intersected with everything at or
/// before `t`. Fails closed with [`CALYX_TIMETRAVEL_BEFORE_HORIZON`] when `t` is
/// before `retention_horizon` (no historical data is read).
pub fn summarize_as_of<S, C>(
    store: &dyn AssocStore,
    scope: Scope,
    t: Ts,
    retention_horizon: Option<Ts>,
    params: Option<SummarizeParams>,
    ctx: &mut SummarizeCtx<'_, S, C>,
) -> Result<SummarizeResult, CalyxError>
where
    S: LedgerCfStore,
    C: Clock,
{
    if let Some(horizon) = retention_horizon
        && t < horizon
    {
        return Err(before_horizon(t, horizon));
    }
    let as_of_scope = Scope::Intersect {
        left: Box::new(scope),
        right: Box::new(Scope::TimeWindow { t0: 0, t1: t }),
    };
    summarize(store, as_of_scope, params, ctx)
}

/// Rejects any inverted `TimeWindow` anywhere in the scope tree (fail-closed).
fn validate_scope(scope: &Scope) -> Result<(), CalyxError> {
    match scope {
        Scope::TimeWindow { t0, t1 } if t0 > t1 => Err(invalid_time_window(*t0, *t1)),
        Scope::Union { left, right } | Scope::Intersect { left, right } => {
            validate_scope(left)?;
            validate_scope(right)
        }
        _ => Ok(()),
    }
}

/// Derives kernel-graph params whose `target_fraction` caps the kernel at ~`max`
/// nodes for this scope's materialized graph (honest sizing — metrics are then
/// computed on the capped kernel, not a post-hoc truncation).
fn target_fraction_for_cap(
    scope: &Scope,
    store: &dyn AssocStore,
    max: usize,
    base: KernelGraphParams,
) -> Result<KernelGraphParams, CalyxError> {
    let node_count = materialize_scope(scope, store)
        .map_err(to_calyx)?
        .node_count();
    if node_count == 0 {
        return Ok(base);
    }
    let fraction = (max as f32 / node_count as f32).clamp(f32::MIN_POSITIVE, 1.0);
    Ok(KernelGraphParams {
        target_fraction: fraction,
        ..base
    })
}

/// Appends the `SUMMARIZE_INVOKED` provenance entry and returns its reference.
fn append_invoked<S, C>(
    ledger: &mut LedgerAppender<S, C>,
    scope_h: &[u8; 32],
    kernel_size: usize,
    kernel_only_recall: f32,
    grounded_fraction: f32,
) -> Result<LedgerRef, CalyxError>
where
    S: LedgerCfStore,
    C: Clock,
{
    let payload = serde_json::to_vec(&serde_json::json!({
        "marker": SUMMARIZE_INVOKED_MARKER,
        "scope_hash": hex32(scope_h),
        "kernel_size": kernel_size,
        "kernel_only_recall": kernel_only_recall,
        "grounded_fraction": grounded_fraction,
    }))
    .map_err(|e| CalyxError {
        code: "CALYX_SUMMARIZE_LEDGER_ENCODE",
        message: format!("failed to encode SUMMARIZE_INVOKED payload: {e}"),
        remediation: "report this bug; summarize payload is a fixed JSON shape",
    })?;
    ledger.append(
        EntryKind::Kernel,
        SubjectId::Kernel(scope_h.to_vec()),
        payload,
        ActorId::Service("calyx-lodestar-summarize".to_string()),
    )
}

/// Maps a [`LodestarError`] to the canonical cross-crate [`CalyxError`], keeping
/// the stable `CALYX_*` code.
fn to_calyx(err: LodestarError) -> CalyxError {
    CalyxError {
        code: err.code(),
        message: err.to_string(),
        remediation: "inspect the scope, corpus, and kernel parameters",
    }
}

fn invalid_time_window(t0: Ts, t1: Ts) -> CalyxError {
    CalyxError {
        code: CALYX_SCOPE_INVALID_TIME_WINDOW,
        message: format!("inverted time window: t0={t0} > t1={t1}"),
        remediation: "supply a TimeWindow scope with t0 <= t1",
    }
}

fn insufficient_grounding(grounded_fraction: f32) -> CalyxError {
    CalyxError {
        code: CALYX_SUMMARIZE_INSUFFICIENT_GROUNDING,
        message: format!(
            "grounded_fraction {grounded_fraction:.6} < required {MIN_GROUNDED_FRACTION}"
        ),
        remediation: "anchor more of the scope or drop require_grounded",
    }
}

fn before_horizon(t: Ts, horizon: Ts) -> CalyxError {
    CalyxError {
        code: CALYX_TIMETRAVEL_BEFORE_HORIZON,
        message: format!("as-of time {t} is before retention horizon {horizon}"),
        remediation: "summarize at or after the retention horizon",
    }
}

/// Lowercase hex of a 32-byte hash (no external dependency).
fn hex32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
