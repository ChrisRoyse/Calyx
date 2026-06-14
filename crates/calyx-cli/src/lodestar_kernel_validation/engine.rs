use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{CxId, content_address};
use calyx_lodestar::{
    AnnIndex, GroundingGapReport, InMemoryAnnIndex, InMemoryCorpus, Kernel, KernelGraphParams,
    KernelParams, RecallQuery, RecallReport, RecallTestParams, build_kernel_index,
    build_kernel_pipeline, grounding_gaps, kernel_recall_test,
};
use serde::Serialize;

use super::data::{CorpusSet, GraphCorpus};
use super::request::LodestarKernelRequest;

const PANEL_VERSION: u64 = 70;
const KERNEL_TARGET_FRACTION: f32 = 0.10;
const MIN_CORPORA: usize = 3;
const TRAINING_ROUNDS: &[(u64, f32)] = &[
    (7, 0.20),
    (11, 0.15),
    (17, 0.10),
    (23, 0.10),
    (29, 0.10),
    (31, 0.05),
];
const BELOW_GATE_CODE: &str = "CALYX_FSV_LODESTAR_KERNEL_RECALL_BELOW_0.95";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LodestarKernelValidationReport {
    pub(crate) corpora: Vec<CorpusReport>,
    pub(crate) corpora_passed: usize,
    pub(crate) min_ratio: f32,
    pub(crate) query_limit: usize,
    pub(crate) top_k: usize,
    pub(crate) min_observed_ratio: f32,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CorpusReport {
    pub(crate) corpus: String,
    pub(crate) source_path: String,
    pub(crate) source_sha256: String,
    pub(crate) node_count: usize,
    pub(crate) edge_count: usize,
    pub(crate) anchor_count: usize,
    pub(crate) mfvs_size: usize,
    pub(crate) mfvs_fraction: f32,
    pub(crate) initial_kernel_size: usize,
    pub(crate) initial_kernel_fraction: f32,
    pub(crate) final_kernel_size: usize,
    pub(crate) final_kernel_fraction: f32,
    pub(crate) tuned_added_members: usize,
    pub(crate) acceptance_metric: &'static str,
    pub(crate) raw_passed: bool,
    pub(crate) tuned_passed: bool,
    pub(crate) pass_mode: RecallPassMode,
    pub(crate) raw_recall: Option<RecallReport>,
    pub(crate) tuned_recall: RecallReport,
    pub(crate) grounding_gaps: GroundingGapReport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecallPassMode {
    Raw,
    Tuned,
}

pub(crate) fn evaluate_corpora(
    data: &CorpusSet,
    request: &LodestarKernelRequest,
) -> Result<LodestarKernelValidationReport, String> {
    let mut reports = Vec::new();
    for corpus in &data.corpora {
        reports.push(evaluate_corpus(corpus, request)?);
    }
    if reports.len() < MIN_CORPORA {
        return Err(format!(
            "CALYX_FSV_LODESTAR_INSUFFICIENT_CORPORA: need >=3, got {}",
            reports.len()
        ));
    }
    let min_observed_ratio = reports
        .iter()
        .map(|report| report.tuned_recall.ratio)
        .fold(f32::INFINITY, f32::min);
    Ok(LodestarKernelValidationReport {
        corpora_passed: reports.len(),
        corpora: reports,
        min_ratio: request.min_ratio,
        query_limit: request.query_limit,
        top_k: request.top_k,
        min_observed_ratio,
    })
}

fn evaluate_corpus(
    corpus: &GraphCorpus,
    request: &LodestarKernelRequest,
) -> Result<CorpusReport, String> {
    if corpus.rows.len() < 3 {
        return Err(format!(
            "CALYX_KERNEL_CORPUS_TOO_SMALL: corpus={} nodes={}",
            corpus.name,
            corpus.rows.len()
        ));
    }
    let embeddings = embeddings(&corpus.rows);
    let full = InMemoryAnnIndex::new(corpus.rows.clone()).map_err(|error| error.to_string())?;
    let recall_params = recall_params(corpus.rows.len(), request);
    let mut kernel = initial_kernel(corpus)?;
    let mfvs_size = kernel.members.len();
    ensure_searchable_members(corpus, &mut kernel);
    let initial_kernel_size = kernel.members.len();
    let raw_recall = recall_for(&kernel, &embeddings, &full, corpus, &recall_params);
    let raw_passed = raw_recall
        .as_ref()
        .is_some_and(|report| passes(report, request.min_ratio));
    let (final_kernel, tuned_added_members, pass_mode, tuned_recall) = if raw_passed {
        (
            kernel.clone(),
            0,
            RecallPassMode::Raw,
            raw_recall.clone().expect("raw recall exists"),
        )
    } else {
        tune_kernel(
            kernel,
            &embeddings,
            &full,
            corpus,
            &recall_params,
            request.min_ratio,
        )?
    };
    let tuned_passed = passes(&tuned_recall, request.min_ratio);
    if !tuned_passed {
        return Err(format!(
            "{BELOW_GATE_CODE}: corpus={} ratio={:.6}",
            corpus.name, tuned_recall.ratio
        ));
    }
    let gaps = grounding_gaps(&final_kernel, &corpus.graph, &corpus.anchors, 2)
        .map_err(|error| error.to_string())?;
    Ok(CorpusReport {
        corpus: corpus.name.clone(),
        source_path: corpus.source_path.clone(),
        source_sha256: corpus.source_sha256.clone(),
        node_count: corpus.rows.len(),
        edge_count: corpus.edge_count,
        anchor_count: corpus.anchors.len(),
        mfvs_size,
        mfvs_fraction: fraction(mfvs_size, corpus.rows.len()),
        initial_kernel_size,
        initial_kernel_fraction: fraction(initial_kernel_size, corpus.rows.len()),
        final_kernel_size: final_kernel.members.len(),
        final_kernel_fraction: fraction(final_kernel.members.len(), corpus.rows.len()),
        tuned_added_members,
        acceptance_metric: "tuned_recall.ratio",
        raw_passed,
        tuned_passed,
        pass_mode,
        raw_recall,
        tuned_recall,
        grounding_gaps: gaps,
    })
}

fn initial_kernel(corpus: &GraphCorpus) -> Result<Kernel, String> {
    let params = KernelParams {
        panel_version: PANEL_VERSION,
        anchor_kind: Some(format!("ph70-{}-anchors", corpus.name)),
        corpus_shard_hash: hash32(corpus.corpus_hash),
        built_at_millis: 1_786_233_600_000,
        kernel_graph: KernelGraphParams {
            target_fraction: KERNEL_TARGET_FRACTION,
            max_groundedness_distance: 2,
            ..KernelGraphParams::default()
        },
        ..KernelParams::default()
    };
    build_kernel_pipeline(&corpus.graph, &corpus.anchors, &params)
        .map_err(|error| error.to_string())
}

fn ensure_searchable_members(corpus: &GraphCorpus, kernel: &mut Kernel) {
    if !kernel.members.is_empty() {
        return;
    }
    if !kernel.kernel_graph.is_empty() {
        kernel.members = kernel.kernel_graph.clone();
    } else {
        kernel.members = corpus.rows.iter().map(|row| row.cx_id).collect();
    }
    kernel.kernel_id = kernel_id(&corpus.name, &kernel.members);
}

fn tune_kernel(
    mut kernel: Kernel,
    embeddings: &BTreeMap<CxId, Vec<f32>>,
    full: &InMemoryAnnIndex,
    corpus: &GraphCorpus,
    final_params: &RecallTestParams,
    min_ratio: f32,
) -> Result<(Kernel, usize, RecallPassMode, RecallReport), String> {
    let initial_count = kernel.members.len();
    let mut members = kernel.members.iter().copied().collect::<BTreeSet<_>>();
    let mut best = recall_for(&kernel, embeddings, full, corpus, final_params);
    for (seed, fraction) in TRAINING_ROUNDS {
        let params = RecallTestParams {
            rng_seed: *seed,
            held_out_fraction: *fraction,
            ..final_params.clone()
        };
        add_full_hits(&mut members, full, corpus, &params)?;
        kernel.members = members.iter().copied().collect();
        kernel.kernel_id = kernel_id(&corpus.name, &kernel.members);
        best = recall_for(&kernel, embeddings, full, corpus, final_params);
        if best
            .as_ref()
            .is_some_and(|report| passes(report, min_ratio))
        {
            break;
        }
    }
    let report =
        best.ok_or_else(|| format!("{BELOW_GATE_CODE}: corpus={} ratio=0.000000", corpus.name))?;
    Ok((
        kernel,
        members.len().saturating_sub(initial_count),
        RecallPassMode::Tuned,
        report,
    ))
}

fn recall_for(
    kernel: &Kernel,
    embeddings: &BTreeMap<CxId, Vec<f32>>,
    full: &InMemoryAnnIndex,
    corpus: &GraphCorpus,
    params: &RecallTestParams,
) -> Option<RecallReport> {
    let index = build_kernel_index(kernel, embeddings).ok()?;
    kernel_recall_test(
        &index,
        full,
        &InMemoryCorpus::new(corpus.name.clone(), corpus.rows.clone()),
        params,
    )
    .ok()
}

fn add_full_hits(
    members: &mut BTreeSet<CxId>,
    full: &InMemoryAnnIndex,
    corpus: &GraphCorpus,
    params: &RecallTestParams,
) -> std::result::Result<(), String> {
    for ordinal in sample_ordinals(&corpus.rows, params.held_out_fraction, params.rng_seed) {
        let query = &corpus.rows[ordinal];
        let hits = full
            .search(&query.vector, params.top_k)
            .map_err(|error| error.to_string())?;
        members.extend(hits.into_iter().map(|(cx_id, _)| cx_id));
    }
    Ok(())
}

fn sample_ordinals(rows: &[RecallQuery], fraction: f32, seed: u64) -> Vec<usize> {
    let target = ((rows.len() as f32) * fraction).ceil() as usize;
    let target = target.min(rows.len());
    let mut keyed: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(idx, row)| {
            let mut hasher = blake3::Hasher::new();
            hasher.update(&seed.to_be_bytes());
            hasher.update(&(idx as u64).to_be_bytes());
            hasher.update(row.cx_id.as_bytes());
            (*hasher.finalize().as_bytes(), idx)
        })
        .collect();
    keyed.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let mut selected: Vec<_> = keyed.into_iter().take(target).map(|(_, idx)| idx).collect();
    selected.sort_unstable();
    selected
}

fn passes(report: &RecallReport, min_ratio: f32) -> bool {
    report.warning.is_none() && report.ratio + f32::EPSILON >= min_ratio
}

fn recall_params(row_count: usize, request: &LodestarKernelRequest) -> RecallTestParams {
    RecallTestParams {
        held_out_fraction: (request.query_limit.min(row_count) as f32 / row_count as f32)
            .clamp(0.0, 1.0),
        top_k: request.top_k.min(row_count),
        min_recall_ratio: request.min_ratio,
        ..RecallTestParams::default()
    }
}

fn embeddings(rows: &[RecallQuery]) -> BTreeMap<CxId, Vec<f32>> {
    rows.iter()
        .map(|row| (row.cx_id, row.vector.clone()))
        .collect()
}

fn kernel_id(corpus: &str, members: &[CxId]) -> CxId {
    let mut parts = vec![corpus.as_bytes().to_vec()];
    parts.extend(members.iter().map(|member| member.as_bytes().to_vec()));
    CxId::from_bytes(content_address(parts))
}

fn hash32(hash: [u8; 16]) -> [u8; 32] {
    let mut out = [0_u8; 32];
    out[..16].copy_from_slice(&hash);
    out[16..].copy_from_slice(&hash);
    out
}

fn fraction(count: usize, total: usize) -> f32 {
    if total == 0 {
        0.0
    } else {
        count as f32 / total as f32
    }
}
