//! Empirical kernel-recall gate over the **computed** FVS kernel, wired into CalyxNative admission
//! (issue #216, split from #82).
//!
//! Issue #82 fixed the epic's core complaint — the grounding-kernel members are now genuinely
//! computed (`kernel_forecast::build_fvs_kernel`: a real minimum feedback vertex set + groundedness
//! selection), not caller-supplied. What was still deferred (#216) is the **empirical recall gate**:
//! previously `superiority::SuperiorityTiers::kernel_recall_ratio` was a caller-supplied number, so
//! the admission path never actually measured how much of a resolved-market corpus is answerable
//! through the computed kernel.
//!
//! This module closes that loop. It takes the resolved-market corpus (the `RecallQuery` rows —
//! `cx_id` + embedding — that Sextant/Lodestar operate on) and the association-graph topology,
//! computes the FVS kernel, then measures — over the **real** corpus with the **real** Lodestar
//! recall engine — the fraction of held-out queries whose top-k neighbourhood the computed kernel
//! still reproduces. That measured ratio is the value that flows into the superiority predicate and
//! gates admissibility at ≥ 0.95. Fail loud: a corpus that does not contain an embedding for every
//! computed kernel member is a hard error (the kernel could not be indexed against the corpus), and
//! a below-floor ratio marks the `kernel` tier failing so the forecast is produced but refused.
//!
//! The persisted [`ComputedKernelRecall`] JSON is the FSV source of truth; the association graph is
//! the graph source of truth for the computed members.

use std::collections::BTreeSet;

use calyx_core::CxId;
use calyx_lodestar::{KernelParams, RecallQuery, RecallReport, RecallTestParams};
use calyx_mincut::{AgreementEdge, FrequencyEntry};
use serde::{Deserialize, Serialize};

use crate::calyx_native::CalyxNativeRequest;
use crate::domain::Domain;
use crate::error::{PolyError, Result};
use crate::kernel_forecast::{FvsKernel, build_fvs_kernel_with_members};
use crate::kernel_recall::{
    DomainKernelRecallInput, DomainKernelRecallReport, POLY_KERNEL_RECALL_MIN_RATIO,
    measure_kernel_recall_per_domain,
};

/// Schema tag persisted with every computed-kernel recall measurement.
pub const COMPUTED_KERNEL_RECALL_SCHEMA_VERSION: &str = "poly.computed_kernel_recall.v1";
/// Artifact-kind tag.
pub const COMPUTED_KERNEL_RECALL_ARTIFACT_KIND: &str = "poly_computed_kernel_recall";

/// A computed kernel member has no embedding row in the resolved-market corpus, so the kernel cannot
/// be indexed against it and recall is undefined. Fail closed rather than silently drop the member.
pub const ERR_KERNEL_MEMBER_NOT_IN_CORPUS: &str = "CALYX_POLY_KERNEL_MEMBER_NOT_IN_CORPUS";
/// The computed kernel had zero members, so there is nothing to measure recall against.
pub const ERR_KERNEL_EMPTY_MEMBERS: &str = "CALYX_POLY_KERNEL_EMPTY_MEMBERS";

/// The empirical recall of the computed FVS kernel over a resolved-market corpus, plus the computed
/// kernel it was measured against — the durable admission-input source of truth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComputedKernelRecall {
    /// Schema tag.
    pub schema_version: String,
    /// Artifact-kind tag.
    pub artifact_kind: String,
    /// Domain the corpus/kernel belong to.
    pub domain: Domain,
    /// The computed grounding kernel + true MFVS (graph source of truth for the members).
    pub fvs_kernel: FvsKernel,
    /// Number of resolved-market rows in the corpus the recall was measured over.
    pub corpus_len: usize,
    /// The measured recall ratio (mean recall@k of the computed kernel vs the full index over the
    /// held-out probes) — the empirically-measured admission input, never caller-supplied.
    pub measured_ratio: f64,
    /// Policy floor the ratio is gated at (0.95).
    pub min_ratio: f64,
    /// Whether the measured ratio cleared the floor.
    pub gate_passed: bool,
    /// Number of held-out queries the recall was measured over.
    pub n_queries_tested: usize,
    /// The full Lodestar recall report (graph/recall source of truth).
    pub recall: RecallReport,
    /// Human-readable provenance of the source of truth.
    pub source_of_truth: String,
}

impl ComputedKernelRecall {
    /// The `(kernel_recall_ratio, min_kernel_recall_ratio)` pair to wire into the superiority tiers.
    pub fn superiority_inputs(&self) -> (f64, f64) {
        (self.measured_ratio, self.min_ratio)
    }
}

/// Everything needed to compute the FVS kernel and measure its recall over one domain's corpus.
pub struct ComputedKernelRecallRequest<'a> {
    /// Domain the corpus/kernel belong to.
    pub domain: Domain,
    /// Resolved-market corpus rows (`cx_id` + embedding vector). Must contain a row for every
    /// computed kernel member.
    pub corpus: &'a [RecallQuery],
    /// Association-graph agreement edges the kernel is computed from.
    pub agreements: &'a [AgreementEdge],
    /// Node frequencies the kernel is computed from.
    pub frequencies: &'a [FrequencyEntry],
    /// Grounding anchors for the kernel pipeline.
    pub anchors: &'a [CxId],
    /// Kernel-pipeline parameters.
    pub kernel_params: &'a KernelParams,
    /// Recall-test parameters (held-out fraction, top-k, seed). `min_recall_ratio` is forced to the
    /// Poly policy floor.
    pub recall_params: &'a RecallTestParams,
}

/// Computes the FVS kernel from the association graph, then measures — over the real corpus with the
/// real Lodestar recall engine — the fraction of held-out resolved-market queries answerable through
/// that computed kernel. Fails closed if a computed member has no corpus embedding or the kernel is
/// empty. Does **not** fail closed on a below-floor ratio: it returns the measurement so admission
/// can name the failing tier (see [`produce_calyx_native_forecast_with_measured_kernel_recall`]).
pub fn measure_computed_kernel_recall(
    req: &ComputedKernelRecallRequest<'_>,
) -> Result<ComputedKernelRecall> {
    let (fvs_kernel, member_ids) = build_fvs_kernel_with_members(
        req.agreements,
        req.frequencies,
        req.anchors,
        req.kernel_params,
    )?;

    if member_ids.is_empty() {
        return Err(PolyError::kernel_recall(
            ERR_KERNEL_EMPTY_MEMBERS,
            format!(
                "domain {} computed an empty grounding kernel over a {}-node graph; nothing to \
                 measure recall against",
                req.domain.slug(),
                fvs_kernel.graph_nodes
            ),
        ));
    }

    // Every computed kernel member must have an embedding in the corpus, or the kernel index cannot
    // be built and recall would be silently understated. Fail loud with the exact missing member.
    let corpus_ids: BTreeSet<CxId> = req.corpus.iter().map(|row| row.cx_id).collect();
    for member in &member_ids {
        if !corpus_ids.contains(member) {
            return Err(PolyError::kernel_recall(
                ERR_KERNEL_MEMBER_NOT_IN_CORPUS,
                format!(
                    "domain {} computed kernel member {} has no embedding row in the {}-row \
                     resolved-market corpus; the computed kernel cannot be indexed against the corpus",
                    req.domain.slug(),
                    member,
                    req.corpus.len()
                ),
            ));
        }
    }

    // Force the policy floor so a caller cannot lower the gate below 0.95.
    let mut recall_params = req.recall_params.clone();
    recall_params.min_recall_ratio = POLY_KERNEL_RECALL_MIN_RATIO as f32;

    let input = DomainKernelRecallInput {
        domain: req.domain,
        full_rows: req.corpus.to_vec(),
        kernel_members: member_ids,
        params: recall_params,
    };
    let report = measure_kernel_recall_per_domain(std::slice::from_ref(&input))?;
    let domain_report: &DomainKernelRecallReport = report
        .domains
        .first()
        .expect("measure_kernel_recall_per_domain returns one report per input");

    Ok(ComputedKernelRecall {
        schema_version: COMPUTED_KERNEL_RECALL_SCHEMA_VERSION.to_string(),
        artifact_kind: COMPUTED_KERNEL_RECALL_ARTIFACT_KIND.to_string(),
        domain: req.domain,
        corpus_len: req.corpus.len(),
        measured_ratio: domain_report.recall.ratio as f64,
        min_ratio: POLY_KERNEL_RECALL_MIN_RATIO,
        gate_passed: domain_report.gate_passed,
        n_queries_tested: domain_report.recall.n_queries_tested,
        recall: domain_report.recall.clone(),
        fvs_kernel,
        source_of_truth: "persisted computed-FVS-kernel recall over the resolved-market corpus \
             (Lodestar kernel_recall_test), gated at the 0.95 Poly policy floor"
            .to_string(),
    })
}

/// Overwrites the superiority tiers' kernel-recall inputs with the **empirically measured** ratio, so
/// admissibility depends on measured recall of the computed kernel — not a caller-supplied number.
/// This is the #216 wiring point.
pub fn apply_measured_kernel_recall(req: &mut CalyxNativeRequest, recall: &ComputedKernelRecall) {
    let (ratio, min_ratio) = recall.superiority_inputs();
    req.superiority_tiers.kernel_recall_ratio = ratio;
    req.superiority_tiers.min_kernel_recall_ratio = min_ratio;
}

/// Produces a CalyxNative forecast with the kernel-recall superiority tier driven by the measured
/// recall of the computed kernel. Below the 0.95 floor the `kernel` tier fails and the forecast is
/// produced but marked non-admissible with the failing tier named — fail loud, never silently
/// upgraded.
pub fn produce_calyx_native_forecast_with_measured_kernel_recall(
    mut req: CalyxNativeRequest,
    recall: &ComputedKernelRecall,
    clock: &dyn calyx_core::Clock,
) -> Result<crate::calyx_native::CalyxNativeForecast> {
    apply_measured_kernel_recall(&mut req, recall);
    crate::calyx_native::produce_calyx_native_forecast(&req, clock)
}

/// Persists a computed-kernel recall measurement as JSON and returns its path.
pub fn write_computed_kernel_recall(
    dir: &std::path::Path,
    recall: &ComputedKernelRecall,
) -> Result<std::path::PathBuf> {
    let file_name = format!("computed_kernel_recall_{}.json", recall.domain.slug());
    crate::diagnostics_store::write_json(dir, &file_name, recall)
}

/// Reads a persisted computed-kernel recall measurement back from disk.
pub fn read_computed_kernel_recall(path: &std::path::Path) -> Result<ComputedKernelRecall> {
    crate::diagnostics_store::read_json(path)
}
