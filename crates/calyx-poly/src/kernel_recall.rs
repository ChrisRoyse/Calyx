//! Poly-owned kernel recall gate over Lodestar's recall test.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use calyx_core::CxId;
use calyx_lodestar::{
    EmbeddingStore, GroundednessReport, InMemoryAnnIndex, InMemoryCorpus, Kernel, KernelIndex,
    RecallQuery, RecallReport, RecallTestParams, RecallTestReport, build_kernel_index,
    kernel_recall_gate, kernel_recall_test,
};
use serde::{Deserialize, Serialize};

use crate::domain::Domain;
use crate::error::{PolyError, Result};

/// Poly policy floor for trusting predictions from a domain kernel.
pub const POLY_KERNEL_RECALL_MIN_RATIO: f64 = 0.95;

const POLY_KERNEL_RECALL_MIN_RATIO_F32: f32 = 0.95;

/// Stable success code used in FSV decision ledgers.
pub const POLY_KERNEL_RECALL_GATE_PASSED: &str = "CALYX_POLY_KERNEL_RECALL_GATE_PASSED";

/// One domain's full reference corpus plus the kernel members to verify.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DomainKernelRecallInput {
    pub domain: Domain,
    pub full_rows: Vec<RecallQuery>,
    pub kernel_members: Vec<CxId>,
    pub params: RecallTestParams,
}

/// Persistable recall report for one domain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DomainKernelRecallReport {
    pub domain: Domain,
    pub corpus_name: String,
    pub corpus_len: usize,
    pub kernel_members: usize,
    pub min_recall_ratio: f64,
    pub held_out_fraction: f64,
    pub top_k: usize,
    pub recall: RecallTestReport,
    pub gate_passed: bool,
}

/// Persisted proof that every supplied domain cleared the recall gate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KernelRecallVerificationReport {
    pub schema_version: u32,
    pub source_of_truth: String,
    pub policy_min_recall_ratio: f64,
    pub domain_count: usize,
    pub domains: Vec<DomainKernelRecallReport>,
}

/// Run Lodestar `kernel_recall_gate` for every supplied domain.
pub fn verify_kernel_recall_per_domain(
    inputs: &[DomainKernelRecallInput],
) -> Result<KernelRecallVerificationReport> {
    if inputs.is_empty() {
        return Err(PolyError::kernel_recall(
            "CALYX_POLY_KERNEL_RECALL_NO_DOMAINS",
            "kernel recall verification requires at least one domain input",
        ));
    }

    let mut seen_domains = HashSet::new();
    for input in inputs {
        if !seen_domains.insert(input.domain) {
            return Err(PolyError::kernel_recall(
                "CALYX_POLY_KERNEL_RECALL_DUPLICATE_DOMAIN",
                format!(
                    "domain {} appeared more than once in one recall gate run",
                    input.domain.slug()
                ),
            ));
        }
    }

    let mut domains = Vec::with_capacity(inputs.len());
    for input in inputs {
        domains.push(verify_one_domain(input)?);
    }

    Ok(KernelRecallVerificationReport {
        schema_version: 1,
        source_of_truth:
            "persisted per-domain Lodestar kernel_recall_gate report over exact full-corpus rows"
                .to_string(),
        policy_min_recall_ratio: POLY_KERNEL_RECALL_MIN_RATIO,
        domain_count: domains.len(),
        domains,
    })
}

/// Writes a kernel-recall verification report as the durable source of truth.
pub fn write_kernel_recall_report(
    path: &Path,
    report: &KernelRecallVerificationReport,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            PolyError::kernel_recall(
                "CALYX_POLY_KERNEL_RECALL_REPORT_WRITE",
                format!("create report directory {}: {err}", parent.display()),
            )
        })?;
    }
    let bytes = serde_json::to_vec_pretty(report).map_err(|err| {
        PolyError::kernel_recall(
            "CALYX_POLY_KERNEL_RECALL_REPORT_ENCODE",
            format!("encode kernel recall report: {err}"),
        )
    })?;
    fs::write(path, bytes).map_err(|err| {
        PolyError::kernel_recall(
            "CALYX_POLY_KERNEL_RECALL_REPORT_WRITE",
            format!("write report {}: {err}", path.display()),
        )
    })
}

/// Reads the durable kernel-recall report from disk.
pub fn read_kernel_recall_report(path: &Path) -> Result<KernelRecallVerificationReport> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::kernel_recall(
            "CALYX_POLY_KERNEL_RECALL_REPORT_READ",
            format!("read report {}: {err}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        PolyError::kernel_recall(
            "CALYX_POLY_KERNEL_RECALL_REPORT_DECODE",
            format!("decode report {}: {err}", path.display()),
        )
    })
}

/// Runs Lodestar's **non-gating** recall test (issue #216) for every supplied domain and reports the
/// measured ratio without failing closed below the floor. This is the admission-path measurement:
/// the CalyxNative superiority predicate needs the *empirically measured* kernel-recall ratio wired
/// in as a tier input even when a domain is below the 0.95 floor, so the forecast can be produced and
/// marked non-admissible with the failing tier named — rather than the whole run erroring out. The
/// strict, fail-closed gate remains [`verify_kernel_recall_per_domain`].
pub fn measure_kernel_recall_per_domain(
    inputs: &[DomainKernelRecallInput],
) -> Result<KernelRecallVerificationReport> {
    if inputs.is_empty() {
        return Err(PolyError::kernel_recall(
            "CALYX_POLY_KERNEL_RECALL_NO_DOMAINS",
            "kernel recall measurement requires at least one domain input",
        ));
    }
    let mut seen_domains = HashSet::new();
    for input in inputs {
        if !seen_domains.insert(input.domain) {
            return Err(PolyError::kernel_recall(
                "CALYX_POLY_KERNEL_RECALL_DUPLICATE_DOMAIN",
                format!(
                    "domain {} appeared more than once in one recall measurement run",
                    input.domain.slug()
                ),
            ));
        }
    }
    let mut domains = Vec::with_capacity(inputs.len());
    for input in inputs {
        domains.push(measure_one_domain(input)?);
    }
    Ok(KernelRecallVerificationReport {
        schema_version: 1,
        source_of_truth:
            "persisted per-domain Lodestar kernel_recall_test report (measured ratio, non-gating) \
             over exact full-corpus rows"
                .to_string(),
        policy_min_recall_ratio: POLY_KERNEL_RECALL_MIN_RATIO,
        domain_count: domains.len(),
        domains,
    })
}

/// Builds the kernel/full/corpus indices for one domain's recall run. Shared by the strict gate and
/// the non-gating measurement so both exercise the exact same corpus and computed-kernel index.
fn build_domain_recall_indices(
    input: &DomainKernelRecallInput,
) -> Result<(KernelIndex, InMemoryAnnIndex, InMemoryCorpus, String)> {
    let embeddings = RecallEmbeddingStore::from_rows(&input.full_rows);
    let kernel = kernel_for(input)?;
    let kernel_index = build_kernel_index(&kernel, &embeddings)
        .map_err(|err| lodestar_error(input.domain, err))?;
    let full_index = InMemoryAnnIndex::new(input.full_rows.clone())
        .map_err(|err| lodestar_error(input.domain, err))?;
    let corpus_name = format!("poly:{}:kernel-recall", input.domain.slug());
    let corpus = InMemoryCorpus::new(corpus_name.clone(), input.full_rows.clone());
    Ok((kernel_index, full_index, corpus, corpus_name))
}

fn verify_one_domain(input: &DomainKernelRecallInput) -> Result<DomainKernelRecallReport> {
    validate_poly_policy(input)?;
    validate_unique_ids(input)?;

    let (kernel_index, full_index, corpus, corpus_name) = build_domain_recall_indices(input)?;
    let recall = kernel_recall_gate(&kernel_index, &full_index, &corpus, &input.params)
        .map_err(|err| lodestar_error(input.domain, err))?;

    // Reaching here proves `kernel_recall_gate` returned `Ok`, i.e. `enforce_recall_gate` accepted
    // the ratio (`ratio >= min_recall_ratio`); it returns `Err(RecallBelowGate)` otherwise. So the
    // gate verdict is genuinely derived, not asserted — cross-check it explicitly rather than trust a
    // literal.
    let gate_passed = recall.ratio >= input.params.min_recall_ratio;
    debug_assert!(
        gate_passed,
        "kernel_recall_gate returned Ok below the floor"
    );

    Ok(DomainKernelRecallReport {
        domain: input.domain,
        corpus_name,
        corpus_len: input.full_rows.len(),
        kernel_members: input.kernel_members.len(),
        min_recall_ratio: ratio_for_report(input.params.min_recall_ratio),
        held_out_fraction: ratio_for_report(input.params.held_out_fraction),
        top_k: input.params.top_k,
        recall,
        gate_passed,
    })
}

fn measure_one_domain(input: &DomainKernelRecallInput) -> Result<DomainKernelRecallReport> {
    validate_poly_policy(input)?;
    validate_unique_ids(input)?;

    let (kernel_index, full_index, corpus, corpus_name) = build_domain_recall_indices(input)?;
    // Non-gating: always returns the measured report so the ratio can be read even below the floor.
    let recall = kernel_recall_test(&kernel_index, &full_index, &corpus, &input.params)
        .map_err(|err| lodestar_error(input.domain, err))?;
    let gate_passed = recall.ratio >= input.params.min_recall_ratio;

    Ok(DomainKernelRecallReport {
        domain: input.domain,
        corpus_name,
        corpus_len: input.full_rows.len(),
        kernel_members: input.kernel_members.len(),
        min_recall_ratio: ratio_for_report(input.params.min_recall_ratio),
        held_out_fraction: ratio_for_report(input.params.held_out_fraction),
        top_k: input.params.top_k,
        recall,
        gate_passed,
    })
}

fn validate_poly_policy(input: &DomainKernelRecallInput) -> Result<()> {
    if input.params.min_recall_ratio < POLY_KERNEL_RECALL_MIN_RATIO_F32 {
        return Err(PolyError::kernel_recall(
            "CALYX_POLY_KERNEL_RECALL_MIN_BELOW_POLICY",
            format!(
                "domain {} configured min_recall_ratio {:.6}, below Poly policy {:.6}",
                input.domain.slug(),
                input.params.min_recall_ratio,
                POLY_KERNEL_RECALL_MIN_RATIO_F32
            ),
        ));
    }
    // Determinism precondition: Lodestar treats `rng_seed == 0` as a sentinel meaning "seed the
    // held-out draw from the wall clock", which makes the gate verdict and its persisted "durable
    // proof" time-dependent and non-reproducible. A trust gate must be deterministic, so fail
    // closed on the sentinel and require a fixed nonzero seed (issue #183).
    if input.params.rng_seed == 0 {
        return Err(PolyError::kernel_recall(
            "CALYX_POLY_KERNEL_RECALL_NONDETERMINISTIC_SEED",
            format!(
                "domain {} kernel-recall gate requires a fixed nonzero rng_seed; 0 is the Lodestar \
                 wall-clock sentinel and makes the held-out draw and the persisted proof \
                 non-reproducible. Set RecallTestParams.rng_seed to a fixed nonzero integer.",
                input.domain.slug()
            ),
        ));
    }
    Ok(())
}

fn validate_unique_ids(input: &DomainKernelRecallInput) -> Result<()> {
    let mut full = HashSet::new();
    for row in &input.full_rows {
        if !full.insert(row.cx_id) {
            return Err(PolyError::kernel_recall(
                "CALYX_POLY_KERNEL_RECALL_DUPLICATE_CX",
                format!(
                    "domain {} full corpus repeats cx_id {}",
                    input.domain.slug(),
                    row.cx_id
                ),
            ));
        }
    }

    let mut members = HashSet::new();
    for member in &input.kernel_members {
        if !members.insert(*member) {
            return Err(PolyError::kernel_recall(
                "CALYX_POLY_KERNEL_RECALL_DUPLICATE_MEMBER",
                format!(
                    "domain {} kernel repeats member {}",
                    input.domain.slug(),
                    member
                ),
            ));
        }
    }
    Ok(())
}

fn kernel_for(input: &DomainKernelRecallInput) -> Result<Kernel> {
    let corpus_hash = corpus_hash(&input.full_rows)?;
    Ok(Kernel {
        kernel_id: CxId::from_input(
            format!("poly:{}:kernel-recall", input.domain.slug()).as_bytes(),
            136,
            &corpus_hash,
        ),
        panel_version: 136,
        anchor_kind: Some("poly_kernel_recall".to_string()),
        corpus_shard_hash: corpus_hash,
        members: input.kernel_members.clone(),
        kernel_graph: input.kernel_members.clone(),
        groundedness: GroundednessReport {
            reached_anchor: 1.0,
            unanchored_members: Vec::new(),
        },
        recall: RecallReport::default(),
        built_at_millis: 0,
        estimator_provenance: "poly.kernel_recall.v1:lodestar.kernel_recall_gate".to_string(),
        warnings: Vec::new(),
    })
}

fn corpus_hash(rows: &[RecallQuery]) -> Result<[u8; 32]> {
    let bytes = serde_json::to_vec(rows).map_err(|err| {
        PolyError::kernel_recall(
            "CALYX_POLY_KERNEL_RECALL_INPUT_HASH",
            format!("encode recall corpus fingerprint: {err}"),
        )
    })?;
    Ok(*blake3::hash(&bytes).as_bytes())
}

fn lodestar_error(domain: Domain, err: calyx_lodestar::LodestarError) -> PolyError {
    PolyError::kernel_recall(
        err.code(),
        format!(
            "domain {} Lodestar recall gate failed: {err}",
            domain.slug()
        ),
    )
}

fn ratio_for_report(value: f32) -> f64 {
    ((value as f64) * 1_000_000.0).round() / 1_000_000.0
}

struct RecallEmbeddingStore {
    rows: BTreeMap<CxId, Vec<f32>>,
}

impl RecallEmbeddingStore {
    fn from_rows(rows: &[RecallQuery]) -> Self {
        Self {
            rows: rows
                .iter()
                .map(|row| (row.cx_id, row.vector.clone()))
                .collect(),
        }
    }
}

impl EmbeddingStore for RecallEmbeddingStore {
    fn embedding(&self, cx_id: CxId) -> calyx_lodestar::Result<Option<Vec<f32>>> {
        Ok(self.rows.get(&cx_id).cloned())
    }
}
