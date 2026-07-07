//! Local resolved-market corpus builder for the computed-kernel recall gate (issue #219).
//!
//! Poly is local-only: this assembles, on this machine, the `RecallQuery` corpus (`cx_id` +
//! feature vector) and the between-record association graph that
//! [`crate::kernel_recall_admission::measure_computed_kernel_recall`] measures the computed FVS
//! kernel against. No remote box, no external service.
//!
//! ## Built on Calyx principles
//! - **Encode what is explicit, never embed it.** A Polymarket snapshot's price/spread/volume/
//!   liquidity are exact, meaningful numbers — the meaning is *already there*. So the recall vector
//!   is a deterministic **record vector** (handbook §5.2 #11) built from those fields with
//!   embedder-free transforms (raw for bounded probabilities, `signed_log` for heavy-tailed
//!   magnitudes, a derived turnover ratio), L2-normalized for cosine. **No learned embedder is
//!   invoked** — there is nothing latent to project.
//! - **No-flatten.** This is *not* a flatten of the typed lens panel into one opaque blob (that would
//!   violate the no-flatten rule the panel/guard rely on). It is a single dedicated find-similar
//!   record vector — the same vector the kNN base-rate path (`knn_base_rate`) consumes — kept
//!   distinct from the panel's per-slot bits machinery.
//! - **Fail closed.** A snapshot missing any required recall field, or carrying a non-finite value,
//!   is a hard structured error — never a silent zero-imputation (an absent field is not a zero).
//! - **Grounding.** Every corpus row is a *resolved* market: its outcome anchor is real, so the
//!   recall measured over it is grounded, not provisional.

use std::collections::BTreeSet;

use calyx_core::CxId;
use calyx_lodestar::RecallQuery;
use calyx_mincut::{AgreementEdge, FrequencyEntry};
use serde::{Deserialize, Serialize};

use crate::domain::Domain;
use crate::encode::{l2_normalize, signed_log};
use crate::kernel_recall_admission::{
    ComputedKernelRecall, ComputedKernelRecallRequest, measure_computed_kernel_recall,
    write_computed_kernel_recall,
};
use crate::knn_base_rate::ResolvedExemplar;
use crate::model::{MarketSnapshot, Resolution};
use crate::no_lookahead::validate_snapshot_before_resolution;
use calyx_lodestar::{KernelParams, RecallTestParams};

/// A snapshot whose `condition_id` had no matching resolution (cannot be grounded).
pub const ERR_CORPUS_UNRESOLVED: &str = "CALYX_POLY_CORPUS_UNRESOLVED_MARKET";
/// A snapshot is missing a required recall field, so it cannot be encoded comparably. Fail closed
/// rather than zero-impute an absent signal.
pub const ERR_CORPUS_MISSING_FIELD: &str = "CALYX_POLY_CORPUS_MISSING_RECALL_FIELD";
/// The assembled corpus was empty (no resolved markets to measure recall over).
pub const ERR_CORPUS_EMPTY: &str = "CALYX_POLY_CORPUS_EMPTY";
/// Two distinct snapshots content-addressed to the same `CxId` with conflicting outcomes — a
/// corpus-integrity violation.
pub const ERR_CORPUS_DUPLICATE_CONFLICT: &str = "CALYX_POLY_CORPUS_DUPLICATE_CONFLICT";

/// Dimension of the deterministic record vector (see [`market_record_vector`]).
pub const RECALL_VECTOR_DIM: usize = 6;

/// One resolved market to encode: a snapshot plus the resolution that grounds it.
pub struct ResolvedMarketInput<'a> {
    /// The market observation.
    pub snapshot: &'a MarketSnapshot,
    /// The resolution grounding it. Its `condition_id` must equal the snapshot's.
    pub resolution: &'a Resolution,
}

/// The assembled local resolved-market corpus: the recall-gate rows, the kNN exemplars (identical
/// vectors + the grounded outcome), the derived association graph, and the grounding anchors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedMarketCorpus {
    /// `cx_id` + recall vector rows — the input to `measure_computed_kernel_recall`.
    pub recall_queries: Vec<RecallQuery>,
    /// The same rows with their grounded YES/NO outcome, for the kNN base-rate path.
    pub exemplars: Vec<ResolvedExemplar>,
    /// Between-record agreement edges (cosine over the record vectors, clamped to `[0,1]`).
    pub agreements: Vec<AgreementEdge>,
    /// Per-node frequencies (1.0 each — one observation per resolved market).
    pub frequencies: Vec<FrequencyEntry>,
    /// Grounding anchors: every resolved market is grounded, so all rows are anchors.
    pub anchors: Vec<CxId>,
    /// Panel version the `cx_id`s were content-addressed under.
    pub panel_version: u32,
    /// Cosine threshold used to admit an agreement edge.
    pub agreement_threshold: f32,
}

/// Builds the canonical **record vector** for a market snapshot — the single dedicated find-similar
/// vector shared by the kNN base rate and the recall corpus. Deterministic and embedder-free:
///
/// | dim | field                    | transform                         |
/// |-----|--------------------------|-----------------------------------|
/// | 0   | `price`                  | raw (already a probability in [0,1]) |
/// | 1   | `\|price − 0.5\|`         | distance-from-50 (edge vs. toss-up)  |
/// | 2   | `spread`                 | raw (already small/bounded)          |
/// | 3   | `volume_24h`             | `signed_log` (heavy-tailed)          |
/// | 4   | `liquidity`              | `signed_log` (heavy-tailed)          |
/// | 5   | `volume_24h/(liquidity+1)` | `signed_log` turnover ratio (derived) |
///
/// then L2-normalized for cosine. Fails closed if any required field (`price`, `spread`,
/// `volume_24h`, `liquidity`) is absent or non-finite — an absent signal is never zero-imputed.
pub fn market_record_vector(s: &MarketSnapshot) -> crate::error::Result<Vec<f32>> {
    let price = required(s, "price", s.price)?;
    let spread = required(s, "spread", s.spread)?;
    let volume = required(s, "volume_24h", s.volume_24h)?;
    let liquidity = required(s, "liquidity", s.liquidity)?;

    let turnover = volume / (liquidity + 1.0);
    let mut v = vec![
        price as f32,
        (price - 0.5).abs() as f32,
        spread as f32,
        signed_log(volume) as f32,
        signed_log(liquidity) as f32,
        signed_log(turnover) as f32,
    ];
    debug_assert_eq!(v.len(), RECALL_VECTOR_DIM);
    if v.iter().any(|x| !x.is_finite()) {
        return Err(crate::error::PolyError::diagnostics(
            ERR_CORPUS_MISSING_FIELD,
            format!(
                "market {} produced a non-finite record vector; reject or normalize upstream",
                s.condition_id
            ),
        ));
    }
    l2_normalize(&mut v);
    Ok(v)
}

fn required(s: &MarketSnapshot, field: &str, value: Option<f64>) -> crate::error::Result<f64> {
    match value {
        Some(x) if x.is_finite() => Ok(x),
        Some(_) => Err(crate::error::PolyError::diagnostics(
            ERR_CORPUS_MISSING_FIELD,
            format!("market {} field {field} is non-finite", s.condition_id),
        )),
        None => Err(crate::error::PolyError::diagnostics(
            ERR_CORPUS_MISSING_FIELD,
            format!(
                "market {} is missing required recall field {field}; a resolved-market exemplar \
                 must carry it (no zero-imputation of absent signal)",
                s.condition_id
            ),
        )),
    }
}

/// Assembles the local resolved-market corpus from resolved snapshots. Each `cx_id` is content-
/// addressed exactly as the vault would address it (`CxId::from_input` over the snapshot's canonical
/// identity bytes under `panel_version` + `vault_salt`), so the corpus rows line up with the stored
/// constellations. The between-record association graph is the cosine graph over the record vectors:
/// an edge `(a → b)` with weight `cos(a,b)` is admitted when the cosine ≥ `agreement_threshold`.
///
/// Fails closed on: an unresolved snapshot, a missing recall field, an empty corpus, or two distinct
/// snapshots colliding on one `cx_id` with conflicting outcomes.
pub fn build_resolved_market_corpus(
    inputs: &[ResolvedMarketInput<'_>],
    panel_version: u32,
    vault_salt: &[u8],
    agreement_threshold: f32,
) -> crate::error::Result<ResolvedMarketCorpus> {
    if inputs.is_empty() {
        return Err(crate::error::PolyError::diagnostics(
            ERR_CORPUS_EMPTY,
            "resolved-market corpus requires at least one resolved market",
        ));
    }

    let mut recall_queries: Vec<RecallQuery> = Vec::with_capacity(inputs.len());
    let mut exemplars: Vec<ResolvedExemplar> = Vec::with_capacity(inputs.len());
    let mut seen: BTreeSet<CxId> = BTreeSet::new();

    for input in inputs {
        let s = input.snapshot;
        let r = input.resolution;
        if s.condition_id != r.condition_id {
            return Err(crate::error::PolyError::diagnostics(
                ERR_CORPUS_UNRESOLVED,
                format!(
                    "snapshot condition_id {} does not match resolution condition_id {}",
                    s.condition_id, r.condition_id
                ),
            ));
        }
        validate_snapshot_before_resolution(
            s.snapshot_ts,
            r.resolved_ts,
            &format!("resolved-market corpus {}", s.condition_id),
        )?;

        let input_bytes = s.canonical_input_bytes()?;
        let cx_id = CxId::from_input(&input_bytes, panel_version, vault_salt);
        let vector = market_record_vector(s)?;
        let outcome_yes = r.winning_outcome_index == s.outcome_index;

        if !seen.insert(cx_id) {
            // Same content address seen twice: idempotent only if the outcome agrees.
            if let Some(prev) = exemplars.iter().find(|e| e.cx_id == cx_id)
                && prev.outcome_yes != outcome_yes
            {
                return Err(crate::error::PolyError::diagnostics(
                    ERR_CORPUS_DUPLICATE_CONFLICT,
                    format!(
                        "cx_id {cx_id} appears twice with conflicting resolved outcomes; corpus \
                         integrity violated"
                    ),
                ));
            }
            continue;
        }

        recall_queries.push(RecallQuery {
            cx_id,
            vector: vector.clone(),
        });
        exemplars.push(ResolvedExemplar {
            cx_id,
            vector,
            outcome_yes,
        });
    }

    let anchors: Vec<CxId> = recall_queries.iter().map(|q| q.cx_id).collect();
    let frequencies: Vec<FrequencyEntry> = anchors
        .iter()
        .map(|cx_id| FrequencyEntry {
            cx_id: *cx_id,
            frequency: 1.0,
        })
        .collect();
    let agreements = between_record_agreement_graph(&recall_queries, agreement_threshold);

    Ok(ResolvedMarketCorpus {
        recall_queries,
        exemplars,
        agreements,
        frequencies,
        anchors,
        panel_version,
        agreement_threshold,
    })
}

/// The between-record association graph (handbook §7.3): for every ordered pair of distinct rows,
/// admit a directed agreement edge weighted by their cosine similarity when it clears `threshold`.
/// Vectors are already L2-normalized, so cosine is the dot product; it is clamped to `[0,1]` for a
/// valid edge weight.
pub fn between_record_agreement_graph(rows: &[RecallQuery], threshold: f32) -> Vec<AgreementEdge> {
    let mut edges = Vec::new();
    for (i, a) in rows.iter().enumerate() {
        for (j, b) in rows.iter().enumerate() {
            if i == j {
                continue;
            }
            let cos = cosine(&a.vector, &b.vector);
            if cos >= threshold {
                edges.push(AgreementEdge {
                    src: a.cx_id,
                    dst: b.cx_id,
                    agreement: cos.clamp(0.0, 1.0),
                    directional_confidence: cos.clamp(0.0, 1.0),
                });
            }
        }
    }
    edges
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Parameters for the one-call local recall run.
pub struct LocalRecallRunParams<'a> {
    /// Domain the corpus/kernel belong to.
    pub domain: Domain,
    /// Panel version to content-address `cx_id`s under (must match the vault's panel).
    pub panel_version: u32,
    /// Vault salt for content-addressing.
    pub vault_salt: &'a [u8],
    /// Cosine threshold for admitting a between-record agreement edge.
    pub agreement_threshold: f32,
    /// Kernel-pipeline parameters.
    pub kernel_params: &'a KernelParams,
    /// Recall-test parameters (`min_recall_ratio` is forced to the 0.95 policy floor downstream).
    pub recall_params: &'a RecallTestParams,
    /// Optional directory to persist the `ComputedKernelRecall` JSON into. `None` skips persistence.
    pub persist_dir: Option<&'a std::path::Path>,
}

/// The end-to-end local run: build the resolved-market corpus, compute the FVS kernel, measure its
/// empirical recall over the corpus with the real Lodestar engine, and (optionally) persist the
/// report. This is the single entry point the production run uses once resolved markets are ingested
/// locally — no remote box, no learned embedder. Returns the built corpus alongside the measurement
/// so the caller can wire the ratio into admission via
/// [`crate::kernel_recall_admission::apply_measured_kernel_recall`].
pub fn run_local_computed_kernel_recall(
    inputs: &[ResolvedMarketInput<'_>],
    params: &LocalRecallRunParams<'_>,
) -> crate::error::Result<(ResolvedMarketCorpus, ComputedKernelRecall)> {
    let corpus = build_resolved_market_corpus(
        inputs,
        params.panel_version,
        params.vault_salt,
        params.agreement_threshold,
    )?;
    let recall = measure_computed_kernel_recall(&ComputedKernelRecallRequest {
        domain: params.domain,
        corpus: &corpus.recall_queries,
        agreements: &corpus.agreements,
        frequencies: &corpus.frequencies,
        anchors: &corpus.anchors,
        kernel_params: params.kernel_params,
        recall_params: params.recall_params,
    })?;
    if let Some(dir) = params.persist_dir {
        write_computed_kernel_recall(dir, &recall)?;
    }
    Ok((corpus, recall))
}

#[cfg(test)]
#[path = "resolved_market_corpus_tests.rs"]
mod tests;
