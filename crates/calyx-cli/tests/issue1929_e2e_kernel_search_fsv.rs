//! Issue #1929 criterion 4 — end-to-end native search/fusion over a persisted
//! segmented-slot vault.
//!
//! This opt-in real-service FSV measures a small corpus of real documents into
//! a two-lane typed panel — a segmented E5 lane (`SlotVector::SegmentedMulti`,
//! slot 9) and a dense E5 lead lane (`SlotVector::Dense`, slot 8) — persists
//! every constellation into a durable `AsterVault`, reopens the vault cold from
//! disk, and runs the production typed-block RRF ranker
//! (`rank_typed_panel_candidates`, the same fusion the live search/kernel-answer
//! paths use) over the DB-native panels. It proves that:
//!
//! * a known-item query ranks its own persisted constellation first;
//! * the segmented lane's per-lane evidence carries segment offsets and
//!   directional block matches (late interaction, never a pooled vector);
//! * every number is read back from persisted state, not in-process;
//! * a dense-only consumer can never receive a pooled surrogate — the persisted
//!   segmented slot's `as_dense()` is `None`.
//!
//! Run it against the frozen E5 service with a corpus directory of real
//! opinions (one file per document; include the 59,933-byte Cuyahoga opinion):
//! ```text
//! CALYX_FSV_1929_TEI=http://127.0.0.1:18190/embed \
//! CALYX_FSV_1929_E2E_CORPUS=/path/to/opinions_dir \
//! CALYX_FSV_1929_OUT=/var/lib/calyx/fsv/issue1929-e2e \
//! cargo test -p calyx-cli --test issue1929_e2e_kernel_search_fsv -- --nocapture
//! ```
//! Without `CALYX_FSV_1929_TEI` + `CALYX_FSV_1929_E2E_CORPUS` the test is
//! skipped (CI has no TEI service).

use std::collections::BTreeMap;
use std::path::PathBuf;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, Input, InputRef, LedgerRef, Lens, Modality, SlotId, SlotVector,
    VaultId, VaultStore,
};
use calyx_lodestar::{TypedPanelVectors, rank_typed_panel_candidates};
use calyx_registry::{DEFAULT_TEI_SEGMENTATION_POLICY, TeiHttpLens, TeiSegmentedLens};
use serde_json::{Value, json};

const DIM: u32 = 768;
const DENSE_SLOT: u16 = 8;
const SEG_SLOT: u16 = 9;
const SALT: &[u8] = b"issue1929-e2e-kernel-search-salt";
const DENSE_HEAD_BYTES: usize = 1_200;

fn tei_endpoint() -> Option<String> {
    std::env::var("CALYX_FSV_1929_TEI").ok()
}

fn corpus_dir() -> Option<PathBuf> {
    std::env::var("CALYX_FSV_1929_E2E_CORPUS")
        .ok()
        .map(PathBuf::from)
}

fn out_dir() -> PathBuf {
    std::env::var("CALYX_FSV_1929_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("issue1929-e2e"))
}

/// Largest UTF-8 char boundary at or below `max_bytes`.
fn head(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn source_hash(measured: &SlotVector) -> [u8; 32] {
    match measured {
        SlotVector::SegmentedMulti { provenance, .. } => provenance.source_hash,
        other => panic!("expected SegmentedMulti, got {other:?}"),
    }
}

fn hex8(bytes: &[u8; 32]) -> String {
    bytes[..4].iter().map(|b| format!("{b:02x}")).collect()
}

fn write_evidence(name: &str, value: &Value) {
    let dir = out_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    println!("ISSUE1929_E2E_EVIDENCE={}", path.display());
}

struct Measured {
    cx_id: CxId,
    panel: TypedPanelVectors,
    source_hash: [u8; 32],
}

#[test]
fn issue1929_e2e_native_kernel_search_over_segmented_vault_fsv() {
    let (Some(endpoint), Some(corpus)) = (tei_endpoint(), corpus_dir()) else {
        eprintln!("SKIP issue1929 e2e FSV: set CALYX_FSV_1929_TEI and CALYX_FSV_1929_E2E_CORPUS");
        return;
    };

    let seg_lens = TeiSegmentedLens::new(
        "issue1929-e5-segmented",
        &endpoint,
        Modality::Text,
        DIM,
        DEFAULT_TEI_SEGMENTATION_POLICY,
    )
    .expect("segmented lens");
    let dense_lens = TeiHttpLens::new("issue1929-e5-dense-lead", &endpoint, Modality::Text, DIM);

    // Load a small corpus of real documents (one file per document).
    let mut files = std::fs::read_dir(&corpus)
        .expect("read corpus dir")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort();
    assert!(
        files.len() >= 3,
        "criterion 4 needs a small real corpus (>=3 docs); got {}",
        files.len()
    );

    let dense_slot = SlotId::new(DENSE_SLOT);
    let seg_slot = SlotId::new(SEG_SLOT);
    let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let dir = out_dir().join("vault");
    let _ = std::fs::remove_dir_all(&dir);

    // Measure both lanes for every document and persist a complete typed panel.
    let mut measured = Vec::with_capacity(files.len());
    {
        let vault = AsterVault::new_durable(&dir, vault_id, SALT.to_vec(), VaultOptions::default())
            .unwrap();
        for path in &files {
            let bytes = std::fs::read(path).expect("read doc");
            let text = String::from_utf8(bytes.clone()).expect("utf8 doc");
            let seg = seg_lens
                .measure(&Input::new(Modality::Text, bytes.clone()))
                .expect("segmented measure");
            assert!(
                matches!(seg, SlotVector::SegmentedMulti { .. }),
                "segmented lane must be SegmentedMulti"
            );
            let dense = dense_lens
                .measure(&Input::new(
                    Modality::Text,
                    head(&text, DENSE_HEAD_BYTES).as_bytes().to_vec(),
                ))
                .expect("dense lead measure");
            assert!(
                matches!(dense, SlotVector::Dense { .. }),
                "dense lane must be Dense"
            );
            let hash = source_hash(&seg);
            let cx_id = CxId::from_input(&bytes, 1, SALT);
            let panel: TypedPanelVectors =
                BTreeMap::from([(dense_slot, dense.clone()), (seg_slot, seg.clone())]);
            let cx = Constellation {
                cx_id,
                vault_id,
                panel_version: 1,
                created_at: 1_785_600_000,
                input_ref: InputRef {
                    hash,
                    pointer: None,
                    redacted: false,
                },
                modality: Modality::Text,
                slots: panel.clone(),
                scalars: BTreeMap::new(),
                metadata: BTreeMap::new(),
                anchors: Vec::new(),
                provenance: LedgerRef {
                    seq: 0,
                    hash: [0u8; 32],
                },
                flags: CxFlags {
                    ungrounded: true,
                    ..CxFlags::default()
                },
            };
            vault.put(cx).expect("durable put panel");
            measured.push(Measured {
                cx_id,
                panel,
                source_hash: hash,
            });
        }
        vault.flush().expect("flush");
    }

    // Cold reopen: read every panel back DB-native.
    let reopened =
        AsterVault::new_durable(&dir, vault_id, SALT.to_vec(), VaultOptions::default()).unwrap();
    let snapshot = reopened.snapshot();
    let mut candidates: BTreeMap<CxId, TypedPanelVectors> = BTreeMap::new();
    for m in &measured {
        let read = reopened
            .get(m.cx_id, snapshot)
            .expect("read back constellation from disk");
        let dense = read
            .slots
            .get(&dense_slot)
            .expect("dense slot present")
            .clone();
        let seg = read
            .slots
            .get(&seg_slot)
            .expect("segmented slot present")
            .clone();
        // DB-native readback is byte-exact against what we persisted.
        assert_eq!(
            &dense,
            m.panel.get(&dense_slot).unwrap(),
            "dense readback exact"
        );
        assert_eq!(
            &seg,
            m.panel.get(&seg_slot).unwrap(),
            "segmented readback exact"
        );
        // Anti-silent-fallback: the persisted segmented slot never densifies.
        assert!(
            seg.as_dense().is_none(),
            "segmented slot must never expose a pooled dense surrogate"
        );
        candidates.insert(
            m.cx_id,
            BTreeMap::from([(dense_slot, dense), (seg_slot, seg)]),
        );
    }

    // Known-item queries: every persisted document, ranked against the whole
    // corpus using the production typed_block_rrf ranker (the same fusion the
    // live search/kernel-answer paths use), must rank its own constellation
    // first. Track the most-segmented document for the detailed per-segment
    // lane assertions below.
    let slots = [dense_slot, seg_slot];
    let mut best_seg_doc = 0usize;
    let mut best_seg_count = 0u32;
    let mut per_query = Vec::with_capacity(measured.len());
    for (index, query) in measured.iter().enumerate() {
        let query_panel = candidates.get(&query.cx_id).expect("query panel").clone();
        let hits = rank_typed_panel_candidates(&query_panel, &candidates, &slots)
            .expect("typed_block_rrf ranking");
        assert!(!hits.is_empty(), "ranker must return hits");
        assert_eq!(
            hits[0].cx_id, query.cx_id,
            "known-item query {index} must rank its own persisted constellation first"
        );
        let seg_blocks = hits[0]
            .lanes
            .iter()
            .find(|lane| lane.slot == seg_slot)
            .expect("segmented lane present")
            .similarity
            .left_block_count;
        if seg_blocks > best_seg_count {
            best_seg_count = seg_blocks;
            best_seg_doc = index;
        }
        per_query.push(json!({
            "query_cx": query.cx_id.to_string(),
            "ranked_first": hits[0].cx_id == query.cx_id,
            "top_score": hits[0].score,
            "seg_blocks": seg_blocks,
        }));
    }

    // Detailed per-segment evidence on the most-segmented known item.
    let query = &measured[best_seg_doc];
    let query_panel = candidates.get(&query.cx_id).expect("query panel").clone();
    let hits = rank_typed_panel_candidates(&query_panel, &candidates, &slots)
        .expect("typed_block_rrf ranking");
    let top = &hits[0];
    assert_eq!(top.lanes.len(), 2, "one lane per panel slot");
    let seg_lane = top
        .lanes
        .iter()
        .find(|lane| lane.slot == seg_slot)
        .expect("segmented lane present");
    assert!(
        seg_lane.similarity.left_segmentation.is_some(),
        "segmented lane retains segmentation provenance"
    );
    assert!(
        seg_lane
            .similarity
            .left_blocks
            .iter()
            .all(|block| block.token_start.is_some() && block.byte_start.is_some()),
        "segmented lane blocks carry per-segment byte/token offsets"
    );
    assert_eq!(
        seg_lane.similarity.left_to_right.len() as u32,
        seg_lane.similarity.left_block_count,
        "directional block matches cover every segment"
    );
    assert!(
        seg_lane.similarity.score > 0.999,
        "self MaxSim on the known item is ~1.0, got {}",
        seg_lane.similarity.score
    );

    let report = json!({
        "corpus_docs": measured.len(),
        "slots": [DENSE_SLOT, SEG_SLOT],
        "per_query_known_item": per_query,
        "detailed_query_cx": query.cx_id.to_string(),
        "detailed_query_source_hash_hex": hex8(&query.source_hash),
        "top_hit_cx": top.cx_id.to_string(),
        "top_hit_score": top.score,
        "known_item_ranked_first": top.cx_id == query.cx_id,
        "seg_lane_block_count": seg_lane.similarity.left_block_count,
        "seg_lane_score": seg_lane.similarity.score,
        "seg_lane_has_segmentation": seg_lane.similarity.left_segmentation.is_some(),
        "seg_lane_pairwise_digest_hex": hex8(&seg_lane.similarity.pairwise_digest),
        "seg_slot_as_dense_is_none": candidates
            .get(&query.cx_id)
            .unwrap()
            .get(&seg_slot)
            .unwrap()
            .as_dense()
            .is_none(),
        "hits": hits.iter().take(5).map(|hit| json!({
            "cx": hit.cx_id.to_string(),
            "score": hit.score,
        })).collect::<Vec<_>>(),
    });
    write_evidence("issue1929-e2e-kernel-search-fsv.json", &report);
}
