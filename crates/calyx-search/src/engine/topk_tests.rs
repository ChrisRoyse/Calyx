//! Engine-level final top-k contract tests (issue #1846).
//!
//! The persisted search engine keeps a WIDE internal retrieval window
//! (`retrieval_k`, historically 64) so rank fusion has recall, but the FINAL
//! result set returned to a caller must be pruned to the caller-requested `k`.
//! Before #1846 the engine truncated the final list to `retrieval_k`, leaking
//! up to 64 hits to callers (e.g. probe-matrix) that requested `top_k=10`.
//!
//! These tests build a REAL vault (real byte-features + sparse algorithmic
//! lenses, real measured constellations from real text, real persisted search
//! index sidecars) with a corpus much larger than `k` and prove every public
//! search entry point returns at most `k` hits for every fusion mode, with
//! deterministic ordering, and that a cache-cold and cache-warm run produce
//! byte-identical bounded output. No GPU, no mock vectors.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Asymmetry, Input, Modality, Panel, QuantPolicy, Slot, SlotId,
    SlotKey, SlotShape, SlotState, VaultId, VaultStore,
};
use calyx_registry::measure::measure_constellation;
use calyx_registry::spec::default_recall_delta;
use calyx_registry::{
    AlgorithmicLens, LensRuntime, LensSpec, Registry, VaultPanelState, load_vault_panel_state,
    persist_vault_panel_state,
};
use ulid::Ulid;

use super::{
    FusionChoice, GuardChoice, SearchBudget, SearchFreshness, SearchSlotCache,
    measure_query_vectors, search_outcome_with_query_vectors_freshness,
    search_outcome_with_query_vectors_freshness_cached,
};
use crate::persisted::rebuild_for_vault;

const DENSE_SLOT: SlotId = SlotId::new(0);
const SPARSE_SLOT: SlotId = SlotId::new(1);

struct CorpusFixture {
    root: PathBuf,
    vault_dir: PathBuf,
    vault_id: VaultId,
    doc_count: usize,
}

impl CorpusFixture {
    /// Build a real vault with `doc_count` distinct grounded documents measured
    /// from real text across a dense (byte-features) and sparse lens.
    fn new(name: &str, doc_count: usize) -> Self {
        let root =
            std::env::temp_dir().join(format!("calyx-issue1846-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create temp root");
        let vault_dir = root.join("vault");
        let vault_id = VaultId::from_ulid(Ulid::new());

        let mut registry = Registry::new();
        let byte_lens = AlgorithmicLens::byte_features("issue1846-byte", Modality::Text);
        let byte_id = register(&mut registry, byte_lens, "issue1846-byte", "byte-features");
        let sparse_lens = AlgorithmicLens::sparse_keywords("issue1846-sparse", Modality::Text, 64);
        let sparse_id = register(
            &mut registry,
            sparse_lens,
            "issue1846-sparse",
            "sparse-keywords:64",
        );
        let panel = Panel {
            version: 1,
            slots: vec![
                slot(DENSE_SLOT, "issue1846-byte", byte_id, SlotShape::Dense(16)),
                slot(
                    SPARSE_SLOT,
                    "issue1846-sparse",
                    sparse_id,
                    SlotShape::Sparse(64),
                ),
            ],
            created_at: 1,
            kernel_ref: None,
            guard_ref: None,
        };
        let vault = AsterVault::new_durable(
            &vault_dir,
            vault_id,
            salt(),
            VaultOptions {
                panel: Some(panel.clone()),
                ..VaultOptions::default()
            },
        )
        .expect("open vault");
        persist_vault_panel_state(&vault_dir, &panel, &registry).expect("persist panel");
        let state = VaultPanelState {
            panel,
            registry,
            registry_snapshot: None,
        };
        for index in 0..doc_count {
            let text =
                format!("alpha frontier document {index} clinical mechanistic marker corpus");
            let mut cx =
                measure_constellation(&vault, &state, Input::new(Modality::Text, text.as_str()), 1)
                    .expect("measure constellation");
            cx.metadata = BTreeMap::from([
                ("source_dataset".to_string(), "issue1846".to_string()),
                ("source_id".to_string(), format!("doc-{index}")),
            ]);
            cx.anchors.push(Anchor {
                kind: AnchorKind::Label("answer".to_string()),
                value: AnchorValue::Text(format!("doc-{index}")),
                source: "issue1846-test".to_string(),
                observed_at: 1,
                confidence: 1.0,
            });
            vault.put(cx).expect("put constellation");
        }
        vault.flush().expect("flush vault");
        rebuild_for_vault(&vault_dir, &vault).expect("rebuild search index");
        drop(vault);
        Self {
            root,
            vault_dir,
            vault_id,
            doc_count,
        }
    }

    fn open(&self) -> AsterVault {
        AsterVault::open(
            &self.vault_dir,
            self.vault_id,
            salt(),
            VaultOptions::default(),
        )
        .expect("open vault")
    }

    fn state(&self) -> VaultPanelState {
        load_vault_panel_state(&self.vault_dir).expect("load panel state")
    }
}

impl Drop for CorpusFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn all_fusion_modes() -> Vec<(&'static str, FusionChoice)> {
    vec![
        ("rrf", FusionChoice::Rrf),
        ("weighted_rrf", FusionChoice::WeightedRrf),
        ("kernel_first", FusionChoice::KernelFirst),
        ("single_lens", FusionChoice::SingleLens),
        ("single_lens_slot", FusionChoice::SingleLensSlot(DENSE_SLOT)),
        ("pipeline", FusionChoice::Pipeline),
    ]
}

fn assert_ranked_and_bounded(hits: &[calyx_sextant::Hit], k: usize, mode: &str) {
    assert!(
        hits.len() <= k,
        "mode {mode}: returned {} hits but requested k={k}",
        hits.len()
    );
    for (index, hit) in hits.iter().enumerate() {
        assert_eq!(
            hit.rank,
            index + 1,
            "mode {mode}: ranks must be dense 1..=len"
        );
        if index > 0 {
            let prev = &hits[index - 1];
            assert!(
                prev.score >= hit.score,
                "mode {mode}: hits must be sorted by descending score",
            );
            if prev.score == hit.score {
                assert!(
                    prev.cx_id < hit.cx_id,
                    "mode {mode}: equal-score ties order by ascending cx_id",
                );
            }
        }
    }
}

/// The core #1846 assertion: with a corpus (50) far larger than `k` (5), every
/// fusion mode returns at most `k` deterministically ranked hits — NOT the wide
/// internal retrieval window.
#[test]
fn final_top_k_is_enforced_for_every_fusion_mode() {
    const K: usize = 5;
    let fixture = CorpusFixture::new("all-modes", 50);
    let vault = fixture.open();
    let state = fixture.state();
    let query_vectors = measure_query_vectors(&state, "alpha").expect("measure query");
    // The query must recall more than k candidates, else the test proves nothing.
    let recall = search_outcome_with_query_vectors_freshness(
        &vault,
        &fixture.vault_dir,
        &query_vectors,
        50,
        FusionChoice::Rrf,
        GuardChoice::Off,
        None,
        None,
        false,
        SearchFreshness::Fresh,
        SearchBudget::disabled(),
        None,
    )
    .expect("wide recall search");
    assert!(
        recall.hits.len() > K,
        "corpus must recall more than k={K} candidates (got {}) for the test to be meaningful",
        recall.hits.len()
    );

    for (mode, fusion) in all_fusion_modes() {
        let outcome = search_outcome_with_query_vectors_freshness(
            &vault,
            &fixture.vault_dir,
            &query_vectors,
            K,
            fusion,
            GuardChoice::Off,
            None,
            None,
            false,
            SearchFreshness::Fresh,
            SearchBudget::disabled(),
            None,
        )
        .unwrap_or_else(|error| panic!("mode {mode} search failed: {error}"));
        assert_ranked_and_bounded(&outcome.hits, K, mode);
        // Provenance survived truncation: hits carry real ledger provenance and
        // hydrated docs are present for each returned hit.
        for hit in &outcome.hits {
            assert!(
                outcome.docs.contains_key(&hit.cx_id),
                "mode {mode}: hit {} must have a hydrated source document",
                hit.cx_id
            );
        }
    }
}

/// A cache-cold and cache-warm run over one shared slot cache must produce
/// byte-identical bounded output (issue #1846 explicit ask).
#[test]
fn cache_cold_and_warm_produce_identical_bounded_output() {
    const K: usize = 5;
    let fixture = CorpusFixture::new("cache-parity", 40);
    let vault = fixture.open();
    let state = fixture.state();
    let query_vectors = measure_query_vectors(&state, "alpha").expect("measure query");

    let mut cache = SearchSlotCache::new();
    let run = |cache: Option<&mut SearchSlotCache>| {
        search_outcome_with_query_vectors_freshness_cached(
            &vault,
            &fixture.vault_dir,
            &query_vectors,
            K,
            FusionChoice::WeightedRrf,
            GuardChoice::Off,
            None,
            None,
            None,
            false,
            SearchFreshness::Fresh,
            SearchBudget::disabled(),
            cache,
            None,
        )
        .expect("cached search")
    };

    let cold = run(Some(&mut cache));
    let warm = run(Some(&mut cache));

    assert!(cold.hits.len() <= K && warm.hits.len() <= K);
    assert_eq!(
        cold.hits.len(),
        warm.hits.len(),
        "cache-cold and cache-warm hit counts must match"
    );
    for (a, b) in cold.hits.iter().zip(warm.hits.iter()) {
        assert_eq!(a.cx_id, b.cx_id, "cold/warm cx_id order must match");
        assert_eq!(a.rank, b.rank, "cold/warm ranks must match");
        assert!(
            a.score.to_bits() == b.score.to_bits(),
            "cold/warm scores must be bit-identical"
        );
    }
}

/// Edge case: k=1 yields at most one hit.
#[test]
fn top_k_one_returns_single_hit() {
    let fixture = CorpusFixture::new("k-one", 30);
    let vault = fixture.open();
    let state = fixture.state();
    let query_vectors = measure_query_vectors(&state, "alpha").expect("measure query");
    for (mode, fusion) in all_fusion_modes() {
        let outcome = search_outcome_with_query_vectors_freshness(
            &vault,
            &fixture.vault_dir,
            &query_vectors,
            1,
            fusion,
            GuardChoice::Off,
            None,
            None,
            false,
            SearchFreshness::Fresh,
            SearchBudget::disabled(),
            None,
        )
        .unwrap_or_else(|error| panic!("mode {mode} search failed: {error}"));
        assert!(
            outcome.hits.len() <= 1,
            "mode {mode}: k=1 returned {} hits",
            outcome.hits.len()
        );
    }
}

/// Edge case: k larger than the corpus is bounded by the corpus size and never
/// exceeds k.
#[test]
fn top_k_larger_than_corpus_is_bounded_by_corpus() {
    let fixture = CorpusFixture::new("k-huge", 12);
    let vault = fixture.open();
    let state = fixture.state();
    let query_vectors = measure_query_vectors(&state, "alpha").expect("measure query");
    let outcome = search_outcome_with_query_vectors_freshness(
        &vault,
        &fixture.vault_dir,
        &query_vectors,
        1000,
        FusionChoice::Rrf,
        GuardChoice::Off,
        None,
        None,
        false,
        SearchFreshness::Fresh,
        SearchBudget::disabled(),
        None,
    )
    .expect("search succeeds");
    assert!(
        outcome.hits.len() <= fixture.doc_count,
        "returned {} hits over corpus size {}",
        outcome.hits.len(),
        fixture.doc_count
    );
    assert!(outcome.hits.len() <= 1000);
}

/// Edge case: an empty corpus yields an empty (bounded) outcome, never an
/// over-budget or error result.
#[test]
fn empty_corpus_returns_empty_bounded_outcome() {
    let fixture = CorpusFixture::new("k-empty", 0);
    let vault = fixture.open();
    let state = fixture.state();
    // measure_query_vectors still succeeds; the index is simply empty.
    let query_vectors = measure_query_vectors(&state, "alpha").expect("measure query");
    let outcome = search_outcome_with_query_vectors_freshness(
        &vault,
        &fixture.vault_dir,
        &query_vectors,
        5,
        FusionChoice::Rrf,
        GuardChoice::Off,
        None,
        None,
        false,
        SearchFreshness::Fresh,
        SearchBudget::disabled(),
        None,
    )
    .expect("empty search succeeds with empty outcome");
    assert!(
        outcome.hits.is_empty(),
        "empty corpus must return zero hits, got {}",
        outcome.hits.len()
    );
}

fn register(
    registry: &mut Registry,
    lens: AlgorithmicLens,
    name: &str,
    runtime_kind: &str,
) -> calyx_core::LensId {
    let contract = lens.contract().clone();
    let lens_id = contract.lens_id();
    let spec = LensSpec {
        name: name.to_string(),
        runtime: LensRuntime::Algorithmic {
            kind: runtime_kind.to_string(),
        },
        output: contract.shape(),
        modality: contract.modality(),
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch: None,
        axis: Some(runtime_kind.to_string()),
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::None,
        truncate_dim: None,
        recall_delta: default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    };
    registry
        .register_frozen_with_spec(lens, contract, spec)
        .expect("register lens");
    lens_id
}

fn slot(id: SlotId, key: &str, lens_id: calyx_core::LensId, shape: SlotShape) -> Slot {
    Slot {
        slot_id: id,
        slot_key: SlotKey::new(id, key),
        lens_id,
        shape,
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some(key.to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: Default::default(),
        state: SlotState::Active,
        added_at_panel_version: 1,
    }
}

fn salt() -> Vec<u8> {
    b"issue1846-search".to_vec()
}
