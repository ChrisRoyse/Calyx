//! Issue #46 - sparse BM25 over question/tags, with learned text embedder gated by deficit proof.
//!
//! Source of truth: persisted question/tag corpus, Sextant inverted-index readback behavior, and
//! persisted panel-sufficiency plus lens-autobuild reports read back from disk.

use std::path::Path;

use calyx_assay::{EnsembleConfig, EnsembleLensInput, EstimateBound, TrustTag};
use calyx_core::{AbsentReason, CxId, SlotId, SlotShape, SlotVector};
use calyx_poly::lens_autobuild::{
    ERR_LENS_AUTOBUILD_NO_ADMISSIBLE, ERR_LENS_AUTOBUILD_NO_DEFICIT, LENS_AUTOBUILD_MIN_GAIN_BITS,
    LensAutobuildRequest, LensAutobuildStatus, LensCandidateMeasurement, LensDeficit,
    read_lens_autobuild_report, require_lens_autobuild_admitted, run_lens_autobuild_report,
};
use calyx_poly::lenses::{SignalLens, default_panel};
use calyx_poly::model::MarketSnapshot;
use calyx_poly::panel_sufficiency::{
    PolyPanelSufficiencyRequest, read_panel_sufficiency_report, run_panel_sufficiency_report,
};
use calyx_poly::{
    QUESTION_BM25_DIM, QUESTION_BM25_KEY, QuestionBm25Lens, compute_question_bm25_vector,
};
use calyx_sextant::{InvertedIndex, SextantIndex};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[path = "support/issue046_question_bm25_fsv_support.rs"]
mod issue046_support;
use issue046_support::{cx_for, snapshot};

const QUESTION_SLOT: SlotId = SlotId::new(16);
const MIN_DOCS: usize = 3;
const MIN_ASSAY_ROWS: usize = 50;
const MIN_PANEL_LENSES: usize = 3;

#[test]
fn issue046_question_bm25_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE046_FSV_ROOT", "poly-issue046-question-bm25");
    reset_dir(&root);

    let corpus = known_truth_corpus();
    let corpus_path = root.join("question_tags_corpus.json");
    write_json(
        &corpus_path,
        &serde_json::to_value(&corpus).expect("corpus json"),
    );

    let panel = default_panel(1, vec!["global".to_string()]);
    assert!(
        panel
            .lenses
            .iter()
            .any(|lens| lens.slot() == QUESTION_SLOT && lens.key() == QUESTION_BM25_KEY)
    );
    assert!(
        !panel
            .lenses
            .iter()
            .any(|lens| lens.key() == "question_embed"),
        "learned text embedder must not be in the default panel"
    );

    let bm25 = prove_question_bm25_retrieval(&root, &corpus);
    let vector_edges = prove_question_vector_edges();
    let deficit = persisted_propose_lens_deficit(&root);
    let embedder_happy = happy_admits_question_embedder(&root, &deficit);
    let embedder_edges = prove_embedder_gate_edges(&root, &deficit);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 46,
        "proof_claim": "Poly's default panel includes deterministic sparse BM25 over market question/tags, while a learned question_embed lens remains absent unless persisted sufficiency deficit evidence and measured bit gain justify appending it.",
        "minimum_sufficient_corpus": {
            "question_tag_documents": MIN_DOCS,
            "sufficiency_rows": MIN_ASSAY_ROWS,
            "panel_lenses_for_deficit": MIN_PANEL_LENSES,
            "deficit_reports": 1,
            "autobuild_reports": 4,
            "why_this_is_sufficient": "three disjoint known-truth documents are the smallest corpus that can prove lexical ranking is semantic rather than insertion-order coincidence; 50 rows and 3 lenses are the smallest local Assay corpus already capable of producing a real propose_lens deficit for the optional embedder gate.",
            "why_smaller_is_insufficient": "one or two documents cannot prove a top-1 ranking against multiple distractors, and fewer than 50 rows or 3 lenses cannot produce the persisted sufficiency deficit used by the gate.",
            "why_larger_is_wasteful": "larger market-scale corpora would repeat the same tokenizer->sparse vector->Sextant BM25 ranking path and the same single-cause embedder admission gates without adding another #46 behavior."
        },
        "source_of_truth": {
            "corpus_path": corpus_path.display().to_string(),
            "question_bm25_slot": QUESTION_SLOT.get(),
            "question_bm25_dim": QUESTION_BM25_DIM,
            "persisted_deficit_source": deficit.source_artifact
        },
        "bm25": bm25,
        "vector_edges": vector_edges,
        "question_embedder_happy_path": embedder_happy,
        "question_embedder_edges": embedder_edges,
        "physical_files": files
    });
    let readback_path = root.join("issue046_question_bm25_fsv_report.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!("ISSUE046_QUESTION_BM25_FSV={}", readback_path.display());
}

fn prove_question_bm25_retrieval(root: &Path, corpus: &[MarketSnapshot]) -> Value {
    let mut index = InvertedIndex::new(QUESTION_SLOT);
    for (idx, snapshot) in corpus.iter().enumerate() {
        let vector = compute_question_bm25_vector(
            snapshot.question.as_deref(),
            &snapshot.tags,
            QUESTION_BM25_DIM,
        );
        assert!(matches!(vector, SlotVector::Sparse { .. }));
        index
            .insert(cx_for(&snapshot.slug), vector, idx as u64 + 1)
            .expect("insert question BM25 vector");
    }
    assert_eq!(index.total_docs(), MIN_DOCS);

    let btc = search_top(&index, "bitcoin above 100000 crypto");
    let fed = search_top(&index, "federal rates economics");
    let nba = search_top(&index, "nba finals sports");
    assert_eq!(btc, cx_for("bitcoin-100k"));
    assert_eq!(fed, cx_for("federal-rates"));
    assert_eq!(nba, cx_for("nba-finals"));

    let tag_only = compute_question_bm25_vector(None, &["federal".to_string()], QUESTION_BM25_DIM);
    assert_eq!(
        index
            .search(&tag_only, 1, None)
            .expect("tag-only BM25 query")[0]
            .cx_id,
        cx_for("federal-rates")
    );

    let direct_lens = QuestionBm25Lens::new(QUESTION_SLOT.get(), QUESTION_BM25_KEY);
    assert_eq!(direct_lens.shape(), SlotShape::Sparse(QUESTION_BM25_DIM));
    assert!(matches!(
        direct_lens.measure(&corpus[0]),
        SlotVector::Sparse {
            dim: QUESTION_BM25_DIM,
            ..
        }
    ));

    let index_artifact = json!({
        "indexed_docs": index.total_docs(),
        "terms": index.term_count(),
        "top_hits": {
            "bitcoin_above_100000_crypto": btc.to_string(),
            "federal_rates_economics": fed.to_string(),
            "nba_finals_sports": nba.to_string()
        }
    });
    let index_path = root.join("question_bm25_index_readback.json");
    write_json(&index_path, &index_artifact);
    json!({
        "indexed_docs": index.total_docs(),
        "term_count": index.term_count(),
        "artifact_path": index_path.display().to_string(),
        "top_hits": index_artifact["top_hits"]
    })
}

fn prove_question_vector_edges() -> Value {
    let missing = compute_question_bm25_vector(None, &[], QUESTION_BM25_DIM);
    assert!(matches!(
        missing,
        SlotVector::Absent {
            reason: AbsentReason::LensUnavailable
        }
    ));

    let punctuation =
        compute_question_bm25_vector(Some("?! --"), &["...".to_string()], QUESTION_BM25_DIM);
    assert!(matches!(
        punctuation,
        SlotVector::Absent {
            reason: AbsentReason::NotApplicable
        }
    ));

    let duplicate_terms =
        compute_question_bm25_vector(Some("bitcoin bitcoin rates"), &[], QUESTION_BM25_DIM);
    let SlotVector::Sparse { entries, .. } = duplicate_terms else {
        panic!("duplicate terms should still produce a sparse vector");
    };
    assert_eq!(entries.len(), 2);
    let norm = entries
        .iter()
        .map(|entry| entry.val * entry.val)
        .sum::<f32>()
        .sqrt();
    assert!((norm - 1.0).abs() < 1.0e-6);

    json!({
        "missing_question_and_tags": "absent_lens_unavailable",
        "punctuation_only": "absent_not_applicable",
        "duplicate_terms": {
            "unique_sparse_entries": entries.len(),
            "l2_norm": norm
        }
    })
}

fn persisted_propose_lens_deficit(root: &Path) -> LensDeficit {
    let dir = root.join("source-deficit");
    let run = run_panel_sufficiency_report(&noise_request(), &dir).expect("sufficiency run");
    let report = read_panel_sufficiency_report(&run.report_path).expect("read sufficiency report");
    assert_eq!(report, run.report);
    assert!(!report.sufficient);
    assert!(report.has_deficit_proposal);
    LensDeficit::from_panel_sufficiency_report(&report, run.report_path.display().to_string())
        .expect("propose_lens deficit")
}

fn happy_admits_question_embedder(root: &Path, deficit: &LensDeficit) -> Value {
    let request = request(
        deficit.clone(),
        vec![candidate(
            root,
            "happy-question-embedder",
            "question_embed",
            0.083,
            0.028,
            TrustTag::Trusted,
            "append_lens_spec",
        )],
    );
    let report = run_and_read(root, "happy-question-embedder", &request);
    require_lens_autobuild_admitted(&report).expect("question embedder admitted");
    assert_eq!(report.status, LensAutobuildStatus::Admitted);
    assert_eq!(report.admitted_count, 1);
    let spec = &report.admitted[0];
    assert_eq!(spec.lens_key, "question_embed");
    assert_eq!(spec.registry_patch_kind, "append_lens_spec");
    assert_eq!(spec.target_slots, deficit.weakest_slots);
    json!({
        "status": report.status,
        "admitted_count": report.admitted_count,
        "lens_key": spec.lens_key,
        "expected_gain_bits": spec.expected_gain_bits,
        "ci_low_bits": spec.ci_low_bits,
        "target_slots": spec.target_slots,
        "decision_hash": report.decision_hash
    })
}

fn prove_embedder_gate_edges(root: &Path, deficit: &LensDeficit) -> Value {
    let duplicate = rejected_edge(
        root,
        "edge-duplicate-question-bm25",
        deficit,
        candidate(
            root,
            "edge-duplicate-question-bm25",
            QUESTION_BM25_KEY,
            0.09,
            0.02,
            TrustTag::Trusted,
            "append_lens_spec",
        ),
    );
    let below_gain = rejected_edge(
        root,
        "edge-question-embedder-below-gain",
        deficit,
        candidate(
            root,
            "edge-question-embedder-below-gain",
            "question_embed",
            0.049,
            0.02,
            TrustTag::Trusted,
            "append_lens_spec",
        ),
    );
    let forbidden = rejected_edge(
        root,
        "edge-question-embedder-forbidden-action",
        deficit,
        candidate(
            root,
            "edge-question-embedder-forbidden-action",
            "question_embed",
            0.09,
            0.02,
            TrustTag::Trusted,
            "submit_order",
        ),
    );

    let mut request = request(
        deficit.clone(),
        vec![candidate(
            root,
            "edge-question-embedder-no-deficit",
            "question_embed",
            0.09,
            0.02,
            TrustTag::Trusted,
            "append_lens_spec",
        )],
    );
    request.deficits.clear();
    let err = run_lens_autobuild_report(&request, &root.join("edge-question-embedder-no-deficit"))
        .expect_err("missing deficit rejected");
    assert_eq!(err.code(), ERR_LENS_AUTOBUILD_NO_DEFICIT);

    json!({
        "duplicate_existing_question_bm25": duplicate,
        "question_embedder_below_gain_floor": below_gain,
        "question_embedder_forbidden_action": forbidden,
        "question_embedder_without_deficit": {
            "code": err.code(),
            "message": err.message()
        }
    })
}

fn rejected_edge(
    root: &Path,
    dir: &str,
    deficit: &LensDeficit,
    candidate: LensCandidateMeasurement,
) -> Value {
    let report = run_and_read(root, dir, &request(deficit.clone(), vec![candidate]));
    assert_eq!(report.status, LensAutobuildStatus::Rejected);
    assert_eq!(report.admitted_count, 0);
    let err = require_lens_autobuild_admitted(&report).expect_err("no admissible lens");
    assert_eq!(err.code(), ERR_LENS_AUTOBUILD_NO_ADMISSIBLE);
    let rejection = &report.rejected[0];
    json!({
        "status": report.status,
        "rejection_code": rejection.code,
        "fail_loud_code": err.code(),
        "lens_key": rejection.lens_key,
        "decision_hash": report.decision_hash
    })
}

fn run_and_read(
    root: &Path,
    dir: &str,
    request: &LensAutobuildRequest,
) -> calyx_poly::LensAutobuildReport {
    let run = run_lens_autobuild_report(request, &root.join(dir)).expect("lens autobuild run");
    let readback = read_lens_autobuild_report(&run.report_path).expect("read lens autobuild");
    assert_eq!(readback, run.report);
    readback
}

fn request(
    deficit: LensDeficit,
    candidates: Vec<LensCandidateMeasurement>,
) -> LensAutobuildRequest {
    LensAutobuildRequest {
        domain: deficit.domain.clone(),
        panel_id: deficit.panel_id.clone(),
        panel_version: deficit.panel_version,
        existing_lens_keys: existing_lens_keys(),
        deficits: vec![deficit],
        candidates,
        min_gain_bits: LENS_AUTOBUILD_MIN_GAIN_BITS,
    }
}

fn candidate(
    root: &Path,
    dir: &str,
    key: &str,
    gain: f32,
    ci_low: f32,
    trust: TrustTag,
    requested_action: &str,
) -> LensCandidateMeasurement {
    let evidence_path = root.join(dir).join(format!("{key}_gain.json"));
    let candidate = LensCandidateMeasurement {
        lens_key: key.to_string(),
        encoder_kind: if key == "question_embed" {
            "text_embedding".to_string()
        } else {
            "bm25_text".to_string()
        },
        source_fields: vec!["question".to_string(), "tags".to_string()],
        measured_gain_bits: gain,
        ci_low_bits: ci_low,
        ci_high_bits: gain + 0.02,
        n_samples: MIN_ASSAY_ROWS,
        trust,
        estimate_bound: Some(EstimateBound::LowerBound),
        evidence_artifact: evidence_path.display().to_string(),
        requested_action: requested_action.to_string(),
    };
    write_json(
        &evidence_path,
        &serde_json::to_value(&candidate).expect("candidate evidence json"),
    );
    candidate
}

fn existing_lens_keys() -> Vec<String> {
    default_panel(1, vec!["global".to_string()])
        .lenses
        .iter()
        .map(|lens| lens.key().to_string())
        .filter(|key| lens_autobuild_key_valid(key))
        .collect()
}

fn lens_autobuild_key_valid(key: &str) -> bool {
    key.chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn noise_request() -> PolyPanelSufficiencyRequest {
    let labels = alternating_labels(MIN_ASSAY_ROWS);
    PolyPanelSufficiencyRequest {
        domain: "poly-question-text".to_string(),
        panel_id: "issue046_question_embedder_gate".to_string(),
        panel_version: 1,
        lenses: paired_noise_lenses(MIN_ASSAY_ROWS),
        labels,
        groups: None,
        config: EnsembleConfig {
            source: "issue046_fsv".to_string(),
            min_gate_lenses: MIN_PANEL_LENSES,
            min_marginal_bits: LENS_AUTOBUILD_MIN_GAIN_BITS,
            max_redundancy: 0.95,
            nmi_bins: 8,
        },
    }
}

fn alternating_labels(n: usize) -> Vec<bool> {
    (0..n).map(|idx| idx % 2 == 0).collect()
}

fn paired_noise_lenses(n: usize) -> Vec<EnsembleLensInput> {
    let mut a = Vec::with_capacity(n);
    let mut b = Vec::with_capacity(n);
    let mut c = Vec::with_capacity(n);
    for idx in 0..n {
        let pair = idx / 2;
        a.push(vec![((pair * 17 + 3) % 11) as f32]);
        b.push(vec![((pair * 7 + 5) % 13) as f32]);
        c.push(vec![((pair * 5 + 1) % 17) as f32]);
    }
    vec![
        EnsembleLensInput::new("noise_a", SlotId::new(1), a),
        EnsembleLensInput::new("noise_b", SlotId::new(2), b),
        EnsembleLensInput::new("noise_c", SlotId::new(3), c),
    ]
}

fn search_top(index: &InvertedIndex, text: &str) -> CxId {
    let query = compute_question_bm25_vector(Some(text), &[], QUESTION_BM25_DIM);
    index.search(&query, 1, None).expect("BM25 search")[0].cx_id
}

fn known_truth_corpus() -> Vec<MarketSnapshot> {
    vec![
        snapshot(
            "bitcoin-100k",
            "Will Bitcoin trade above 100000 dollars before June?",
            &["crypto", "bitcoin"],
        ),
        snapshot(
            "federal-rates",
            "Will the Federal Reserve cut interest rates this year?",
            &["economics", "federal", "rates"],
        ),
        snapshot(
            "nba-finals",
            "Will Boston win the NBA Finals?",
            &["sports", "nba", "basketball"],
        ),
    ]
}
