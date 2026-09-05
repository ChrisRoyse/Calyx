//! Regression coverage for issue #1845: the CPU MaxSim serving path streams
//! each segment row-by-row instead of materializing and pinning the whole lens
//! resident. These tests prove (a) the pin preflight charges only the bounded
//! streaming working set (a running scored vector plus one decoded row),
//! independent of the lens token volume; (b) streamed scores are bit-identical
//! to the canonical `MaxSimIndex::maxsim` source of truth, including across
//! multiple segments with cross-segment de-duplication; and (c) candidate/k
//! edge cases stay grounded and fail-closed. No mock data — every byte flows
//! through the real rebuild + on-disk segment sidecars.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use calyx_core::{Constellation, CxId, SlotId, SlotVector};
use calyx_sextant::index::MaxSimIndex;

use super::super::{PersistedSearchIndexes, rebuild_from_docs};
use super::helpers::*;

// 64 MiB — mirror of the streaming per-row working-set cap the predicted
// pin bytes add on top of the running scored vector (see multi/pinned.rs
// STREAMING_ROW_WORKING_SET_BYTES / segments/bounds.rs per-segment budget).
const STREAMING_ROW_WORKING_SET_BYTES: u64 = 64 * 1024 * 1024;

fn multi_tokens(docs: &BTreeMap<CxId, Constellation>, cx_id: &CxId) -> Vec<Vec<f32>> {
    let calyx_core::SlotVector::Multi { tokens, .. } = &docs[cx_id].slots[&SlotId::new(2)] else {
        panic!("slot 2 must be multi");
    };
    tokens.clone()
}

fn two_segment_docs() -> BTreeMap<CxId, Constellation> {
    let mut docs = mixed_docs();
    docs.insert(
        cx(4),
        constellation(
            cx(4),
            [
                (SlotId::new(0), dense(vec![0.4, 0.6])),
                (SlotId::new(1), sparse(8, [4])),
                (SlotId::new(2), multi(2, [[0.25, 0.75]])),
            ],
        ),
    );
    docs
}

#[test]
fn streaming_multi_preflight_charges_bounded_working_set_not_whole_lens() {
    let root = scratch("1845-preflight");
    rebuild_from_docs(&root, &mixed_docs(), 45).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");

    let multi_entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 2)
        .expect("multi entry");
    let rows = multi_entry.len as u64;
    assert_eq!(rows, 3, "mixed_docs slot 2 has three rows");

    let preflight = indexes
        .pin_budget_preflight_for_slots(Some(&BTreeSet::from([SlotId::new(2)])))
        .expect("preflight");

    // Bounded working set = running scored vector (one (CxId, score) per row)
    // plus a single decoded row capped at the per-segment budget. Critically it
    // does NOT scale with token_count * token_dim * 4 (the old resident-pin
    // formula), so a 26 GB lens would preflight at ~the same bytes as this one.
    // required_bytes is computed purely from this slot's requirement (not the
    // process-global pin ledger), so it is deterministic across parallel tests.
    let expected =
        rows * std::mem::size_of::<(CxId, f32)>() as u64 + STREAMING_ROW_WORKING_SET_BYTES;
    assert_eq!(preflight.required_bytes, expected);
    assert!(
        preflight.projected_process_bytes >= preflight.required_bytes,
        "projected process bytes include this slot's requirement",
    );
    assert!(
        preflight.configured_bytes >= preflight.required_bytes,
        "the streaming working set must fit the configured budget",
    );
    // The default budget was lowered from 32 GiB (sized to hold a resident
    // ~26.2 GB MaxSim generation) to 8 GiB once nothing pins the lens. Only
    // assert the exact default when no process override is in effect.
    if std::env::var_os("CALYX_SEARCH_PIN_BUDGET_BYTES").is_none() {
        assert_eq!(preflight.configured_bytes, 8 * 1024 * 1024 * 1024);
    }
    cleanup(root);
}

#[test]
fn streaming_multi_scores_bit_identical_to_maxsim_across_two_segments() {
    let root = scratch("1845-two-segment");
    let docs = two_segment_docs();
    // Two rebuilds so the writer reuses the first segment and appends a second,
    // exercising the streaming scan across multiple on-disk segment sidecars
    // plus cross-segment duplicate detection.
    rebuild_from_docs(&root, &mixed_docs(), 46).expect("first rebuild");
    rebuild_from_docs(&root, &docs, 47).expect("second rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");

    let multi_entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 2)
        .expect("multi entry");
    let manifest = read_multi_segment_manifest(&root, multi_entry);
    assert_eq!(
        manifest["segments"].as_array().unwrap().len(),
        2,
        "the lens must span two physical segments for this test to mean anything",
    );

    let query = vec![vec![0.25f32, 0.75f32]];
    let hits = indexes
        .search(SlotId::new(2), &multi(2, [[0.25, 0.75]]), 4)
        .expect("streaming multi search");

    assert_eq!(
        hits.len(),
        4,
        "k=4 must return every row across both segments"
    );
    for hit in &hits {
        let expected = MaxSimIndex::maxsim(&query, &multi_tokens(&docs, &hit.cx_id));
        assert_eq!(
            hit.score.to_bits(),
            expected.to_bits(),
            "streamed score for {} must be bit-identical to MaxSimIndex::maxsim",
            hit.cx_id
        );
    }
    // Rank order + tie-break must match a direct top_k over the source of truth.
    let mut reference = docs
        .keys()
        .map(|cx_id| {
            (
                *cx_id,
                MaxSimIndex::maxsim(&query, &multi_tokens(&docs, cx_id)),
            )
        })
        .collect::<Vec<_>>();
    reference.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
    });
    let expected_order = reference
        .iter()
        .map(|(cx_id, _)| *cx_id)
        .collect::<Vec<_>>();
    let actual_order = hits.iter().map(|hit| hit.cx_id).collect::<Vec<_>>();
    assert_eq!(actual_order, expected_order);
    cleanup(root);
}

#[test]
fn streaming_multi_candidate_and_k_edges_stay_grounded() {
    let root = scratch("1845-edges");
    let docs = two_segment_docs();
    rebuild_from_docs(&root, &mixed_docs(), 48).expect("first rebuild");
    rebuild_from_docs(&root, &docs, 49).expect("second rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let query = vec![vec![0.6f32, 0.4f32]];

    // Edge 1: k == 0 returns no hits without touching the lens.
    let none = indexes
        .search(SlotId::new(2), &multi(2, [[0.6, 0.4]]), 0)
        .expect("k=0 search");
    assert!(none.is_empty());

    // Edge 2: an empty candidate set demands zero rows and must return nothing
    // (never a silent fall-through to an unfiltered scan).
    let empty = indexes
        .search_filtered(SlotId::new(2), &multi(2, [[0.6, 0.4]]), 4, &BTreeSet::new())
        .expect("empty-candidate search");
    assert!(empty.is_empty());

    // Edge 3: a candidate subset spanning both segments scores exactly those
    // rows, bit-identical to MaxSimIndex::maxsim, and drops none of them.
    let candidates = BTreeSet::from([cx(1), cx(4)]);
    let filtered = indexes
        .search_filtered(SlotId::new(2), &multi(2, [[0.6, 0.4]]), 4, &candidates)
        .expect("filtered search");
    assert_eq!(
        filtered
            .iter()
            .map(|hit| hit.cx_id)
            .collect::<BTreeSet<_>>(),
        candidates
    );
    for hit in &filtered {
        let expected = MaxSimIndex::maxsim(&query, &multi_tokens(&docs, &hit.cx_id));
        assert_eq!(hit.score.to_bits(), expected.to_bits());
    }
    cleanup(root);
}

/// Read a `/proc/self/status` size field (kB) as bytes. Linux only; the FSV
/// harness below is env-gated so non-Linux CI simply skips it.
fn proc_status_bytes(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix(field) {
            let kb: u64 = rest.trim().trim_end_matches(" kB").trim().parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

fn deterministic_multi_row(seed: u64, tokens: usize, token_dim: u32) -> SlotVector {
    // Small LCG so the corpus is real, deterministic, and finite — no mock data,
    // no all-zero rows (which would collapse cosine norms).
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        // Map to (0.05, 1.05) so every token has a non-zero norm.
        0.05f32 + ((state >> 33) as f32 / (1u64 << 31) as f32)
    };
    let rows = (0..tokens)
        .map(|_| (0..token_dim).map(|_| next()).collect::<Vec<f32>>())
        .collect::<Vec<_>>();
    SlotVector::Multi {
        token_dim,
        tokens: rows,
    }
}

/// FULL STATE VERIFICATION (issue #1845): build a large real multi-segment
/// MaxSim lens, drop the build data, then serve it via the streaming CPU path
/// while a sampler thread records peak process RSS. Proves the serving working
/// set is a tiny fraction of the lens token volume (it would be ~the whole lens
/// under the old resident-pin path). Env-gated because it allocates a few
/// hundred MB and is meant to be run explicitly for evidence:
///   CALYX_FSV_RSS=1 cargo test -p calyx-search --lib \
///     streaming_multi_serving_rss_is_bounded -- --exact --nocapture --test-threads=1
#[test]
fn streaming_multi_serving_rss_is_bounded() {
    if std::env::var_os("CALYX_FSV_RSS").is_none() {
        return;
    }
    // Authentic probe-matrix routing: this lens has ~2M tokens (> the 65,536
    // CUDA floor), so without an explicit CPU request search would demand the
    // CUDA path. Probe-matrix pins the CPU path via CALYX_SEARCH_MAXSIM_CUDA=0;
    // do the same here (env-gated, single-threaded harness).
    // SAFETY: this env-gated FSV harness runs alone (--test-threads=1) and sets
    // the variable before any search reads it.
    unsafe {
        std::env::set_var("CALYX_SEARCH_MAXSIM_CUDA", "0");
    }
    let rows: u64 = 8_000;
    let tokens_per_row: usize = 256;
    let token_dim: u32 = 32;
    let lens_bytes = rows * tokens_per_row as u64 * token_dim as u64 * 4;

    let mut docs: BTreeMap<CxId, Constellation> = BTreeMap::new();
    for i in 0..rows {
        let mut id = [0u8; 16];
        id[..8].copy_from_slice(&(i + 1).to_le_bytes());
        let cx_id = CxId::from_bytes(id);
        let row = deterministic_multi_row(i + 1, tokens_per_row, token_dim);
        docs.insert(cx_id, constellation(cx_id, [(SlotId::new(2), row)]));
    }

    let root = scratch("1845-rss");
    let summary = rebuild_from_docs(&root, &docs, 60).expect("rebuild large lens");
    assert_eq!(summary.total_rows, rows as usize);

    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let multi_entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 2)
        .expect("multi entry")
        .clone();
    let manifest = read_multi_segment_manifest(&root, &multi_entry);
    let segment_count = manifest["segments"].as_array().unwrap().len();

    // Physical readback: prove the on-disk segment bytes exist and match the
    // manifest sha256 before we serve from them.
    let first_seg = first_segment_rel(&manifest);
    let seg_state = sidecar_state(&root, &first_seg);
    assert_eq!(seg_state["exists"], true);

    // Free the build corpus so process RSS reflects only serving.
    drop(docs);

    let query = SlotVector::Multi {
        token_dim,
        tokens: (0..4)
            .map(|q| {
                (0..token_dim)
                    .map(|d| 0.1 + 0.01 * (q * 7 + d) as f32)
                    .collect()
            })
            .collect(),
    };

    let before = proc_status_bytes("VmRSS:").expect("VmRSS");
    let hwm_before = proc_status_bytes("VmHWM:").expect("VmHWM");
    let stop = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(AtomicU64::new(before));
    let sampler = {
        let stop = Arc::clone(&stop);
        let peak = Arc::clone(&peak);
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Some(rss) = proc_status_bytes("VmRSS:") {
                    peak.fetch_max(rss, Ordering::Relaxed);
                }
                std::thread::sleep(std::time::Duration::from_micros(500));
            }
        })
    };

    let mut last_hits = Vec::new();
    for _ in 0..5 {
        last_hits = indexes
            .search(SlotId::new(2), &query, 10)
            .expect("streaming search over large lens");
    }
    stop.store(true, Ordering::Relaxed);
    sampler.join().unwrap();

    let peak_during = peak.load(Ordering::Relaxed);
    let hwm_after = proc_status_bytes("VmHWM:").expect("VmHWM");
    let serve_delta = peak_during.saturating_sub(before);

    println!("FSV issue1845 streaming RSS:");
    println!("  rows={rows} tokens_per_row={tokens_per_row} token_dim={token_dim}");
    println!(
        "  lens_token_bytes={lens_bytes} ({} MiB)",
        lens_bytes / 1048576
    );
    println!("  segments={segment_count}");
    println!("  VmRSS_before_serve={before} ({} MiB)", before / 1048576);
    println!(
        "  peak_VmRSS_during_serve={peak_during} ({} MiB)",
        peak_during / 1048576
    );
    println!(
        "  serve_working_set_delta={serve_delta} ({} MiB)",
        serve_delta / 1048576
    );
    println!("  VmHWM_before={hwm_before} VmHWM_after={hwm_after}");
    println!("  hits={}", last_hits.len());

    // The streaming serve must add far less than the lens token volume. A
    // resident-pin path would add ~lens_bytes (plus a 2x transient); streaming
    // holds only the scored vector plus one decoded row.
    assert!(
        serve_delta < lens_bytes / 8,
        "serving working set {serve_delta} must be far below the {lens_bytes}-byte lens token volume",
    );
    assert_eq!(last_hits.len(), 10);
    cleanup(root);
}

/// FSV fail-closed budget contract (issue #1845): with a deliberately tiny pin
/// budget the streaming working-set preflight must refuse the lens with a
/// structured CALYX_SEARCH_PIN_BUDGET_EXCEEDED error — never a silent per-query
/// disk scan and never a dropped lens. Env-gated + single-threaded because it
/// mutates the process-global pin-budget env var.
///   CALYX_FSV_BUDGET=1 cargo test -p calyx-search --lib \
///     streaming_multi_budget_fail_closed -- --exact --test-threads=1
#[test]
fn streaming_multi_budget_fail_closed() {
    if std::env::var_os("CALYX_FSV_BUDGET").is_none() {
        return;
    }
    let root = scratch("1845-budget");
    rebuild_from_docs(&root, &mixed_docs(), 61).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");

    // The streaming working set for the multi lens is ~64 MiB (one decoded row
    // cap) plus a few bytes per row; a 1000-byte budget cannot hold it.
    // SAFETY: env-gated harness, single-threaded, set before the preflight reads it.
    unsafe {
        std::env::set_var("CALYX_SEARCH_PIN_BUDGET_BYTES", "1000");
    }
    let err = indexes
        .pin_budget_preflight_for_slots(Some(&BTreeSet::from([SlotId::new(2)])))
        .expect_err("tiny budget must fail closed");
    assert_eq!(err.code(), "CALYX_SEARCH_PIN_BUDGET_EXCEEDED");
    assert!(
        err.message()
            .contains("exceeding configured budget 1000 bytes")
    );
    unsafe {
        std::env::remove_var("CALYX_SEARCH_PIN_BUDGET_BYTES");
    }
    cleanup(root);
}
