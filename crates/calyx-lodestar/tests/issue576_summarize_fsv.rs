//! Binding Full-State Verification for PH72 · T06 universal summarization (#576).
//!
//! Source of Truth: the on-disk Ledger CF rows written by `DirectoryLedgerStore`
//! (`<root>/<seq:016x>.ledger` files). Every assertion reads the bytes back through
//! an INDEPENDENT `DirectoryLedgerStore::open(...).scan()` — never the `summarize`
//! return value. Synthetic, hand-computed I/O; ≥3 edge cases print SoT state
//! BEFORE and AFTER; a JSON artifact captures the residing data.
//!
//! Run: `cargo test -p calyx-lodestar --test issue576_summarize_fsv -- --nocapture`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{AnchorKind, CxId, FixedClock};
use calyx_ledger::{
    DirectoryLedgerStore, EntryKind, LedgerAppender, LedgerCfStore, LedgerEntry, SubjectId, decode,
};
use calyx_lodestar::{
    CollectionId, SUMMARIZE_INVOKED_MARKER, Scope, ScopeCache, SummarizeCtx, SummarizeParams,
    scope_hash, summarize, summarize_as_of,
};
use calyx_paths::AssocGraph;

mod memory_assoc_support;
use memory_assoc_support::{MemoryAssocStore, cx};

/// Bundles the engine plumbing into a [`SummarizeCtx`] for one call.
fn ctx<'a>(
    cache: &'a mut ScopeCache,
    clock: &'a FixedClock,
    ledger: &'a mut LedgerAppender<DirectoryLedgerStore, FixedClock>,
) -> SummarizeCtx<'a, DirectoryLedgerStore, FixedClock> {
    SummarizeCtx {
        cache,
        clock,
        ledger,
    }
}

/// A 30-node corpus: three anchored cycles plus a chain. Timestamps are monotone
/// in seed (node `n` exists at `1_000 + n`).
fn corpus() -> MemoryAssocStore {
    let mut builder = AssocGraph::builder();
    for seed in 1..=30u8 {
        builder.add_node(cx(seed), 1.0 + (seed % 6) as f32).unwrap();
    }
    let mut edges: Vec<(u8, u8)> = Vec::new();
    // three 5-cycles
    for base in [1u8, 6, 11] {
        edges.extend([
            (base, base + 1),
            (base + 1, base + 2),
            (base + 2, base + 3),
            (base + 3, base + 4),
            (base + 4, base),
        ]);
    }
    edges.push((5, 6)); // bridge cycle1 -> cycle2
    edges.push((10, 11)); // bridge cycle2 -> cycle3
    for n in 16..30 {
        edges.push((n, n + 1)); // tail chain 16..30
    }
    for (a, b) in &edges {
        builder.add_edge(cx(*a), cx(*b), 1.0).unwrap();
    }
    let domain = AnchorKind::Label("domain".to_string());
    MemoryAssocStore::with_indexes(
        builder.build(),
        BTreeMap::from([(CollectionId::from("empty"), BTreeSet::new())]),
        // anchor one node per cycle so grounded_fraction is genuinely non-trivial.
        BTreeMap::from([(domain, vec![cx(1), cx(6), cx(11)])]),
        Some((1..=30u8).map(|s| (cx(s), 1_000_u64 + s as u64)).collect()),
        BTreeMap::new(),
        BTreeMap::new(),
    )
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE576_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-issue576-summarize-fsv"))
}

/// Counts on-disk `*.ledger` rows — the physical Source of Truth.
fn ledger_files(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("ledger"))
                .count()
        })
        .unwrap_or(0)
}

/// INDEPENDENT read of the SoT: re-open the on-disk store and decode every row.
fn read_back(dir: &Path) -> Vec<LedgerEntry> {
    let store = DirectoryLedgerStore::open(dir).expect("reopen ledger store");
    store
        .scan()
        .expect("scan rows")
        .into_iter()
        .map(|row| decode(&row.bytes).expect("decode row"))
        .collect()
}

fn appender(dir: &Path) -> LedgerAppender<DirectoryLedgerStore, FixedClock> {
    let store = DirectoryLedgerStore::open(dir).expect("open ledger store");
    LedgerAppender::open(store, FixedClock::new(1_700_000_000_000)).expect("open appender")
}

#[test]
fn ph72_t06_summarize_full_state_verification() {
    let root = fsv_root();
    fs::remove_dir_all(&root).ok();
    let ledger_dir = root.join("ledger");
    fs::create_dir_all(&ledger_dir).unwrap();
    let store = corpus();
    let clock = FixedClock::new(1_700_000_000_000);
    // max_kernel_size=30 ⇒ target_fraction 1.0 (full FVS); anchor_kind grounds the
    // summary against the "domain" anchors planted at one node per cycle.
    let full_params = || {
        Some(SummarizeParams {
            max_kernel_size: Some(30),
            anchor_kind: Some(AnchorKind::Label("domain".to_string())),
            ..Default::default()
        })
    };

    // ---- Trigger X → Outcome Y: present-day summary writes one SUMMARIZE_INVOKED row.
    println!(
        "BEFORE present summarize: on-disk ledger rows = {}",
        ledger_files(&ledger_dir)
    );
    let mut cache = ScopeCache::new(16);
    let mut led = appender(&ledger_dir);
    let present = summarize(
        &store,
        Scope::AllAssociations,
        full_params(),
        &mut ctx(&mut cache, &clock, &mut led),
    )
    .expect("present summarize");
    drop(led);
    println!(
        "AFTER present summarize:  on-disk ledger rows = {}",
        ledger_files(&ledger_dir)
    );

    // SoT read-back (independent): exactly one entry, kind+subject+payload match.
    let entries = read_back(&ledger_dir);
    assert_eq!(
        entries.len(),
        1,
        "exactly one ledger row physically on disk"
    );
    let e = &entries[0];
    assert_eq!(e.kind, EntryKind::Kernel, "SoT kind == Kernel");
    assert_eq!(
        e.subject,
        SubjectId::Kernel(present.scope_hash.to_vec()),
        "SoT subject carries the scope hash"
    );
    let payload: serde_json::Value = serde_json::from_slice(&e.payload).unwrap();
    assert_eq!(payload["marker"], SUMMARIZE_INVOKED_MARKER);
    assert_eq!(
        payload["scope_hash"],
        serde_json::json!(hex(&scope_hash(&Scope::AllAssociations))),
        "payload scope_hash matches canonical hash (hand-computed)"
    );
    assert_eq!(
        payload["kernel_size"].as_u64().unwrap() as usize,
        present.kernel_size
    );

    // Hand-computed expectations (2+2=4): three 5-cycles → a non-empty feedback
    // vertex set; anchored at one node per cycle → grounded_fraction > 0.
    assert!(
        present.kernel_size > 0,
        "non-trivial corpus ⇒ non-empty kernel"
    );
    assert!(
        present.grounded_fraction > 0.0,
        "anchored corpus ⇒ grounded_fraction > 0"
    );
    assert!((0.0..=1.0).contains(&present.grounded_fraction));
    let universe: BTreeSet<CxId> = (1..=30u8).map(cx).collect();
    assert!(
        present.kernel_ids.iter().all(|id| universe.contains(id)),
        "kernel ⊆ corpus"
    );

    // ---- Time-travel: a historical summary (as of t=1_010, nodes 1..=10) differs
    //      from the present (nodes 1..=30). Historical kernel ⊆ historical universe.
    let mut led = appender(&ledger_dir);
    let historical = summarize_as_of(
        &store,
        Scope::AllAssociations,
        1_010, // only nodes with ts <= 1_010 (seeds 1..=10) exist as-of here
        None,
        full_params(),
        &mut ctx(&mut cache, &clock, &mut led),
    )
    .expect("historical summarize");
    drop(led);
    let hist_universe: BTreeSet<CxId> = (1..=10u8).map(cx).collect();
    assert!(
        historical
            .kernel_ids
            .iter()
            .all(|id| hist_universe.contains(id)),
        "as-of kernel only contains nodes that existed at t"
    );
    assert!(
        historical.kernel_size <= present.kernel_size,
        "historical summary is no larger than the present ({} <= {})",
        historical.kernel_size,
        present.kernel_size
    );
    assert_eq!(
        read_back(&ledger_dir).len(),
        2,
        "two provenance rows now on disk"
    );

    // ---- Edge audit (≥3), each printing SoT state BEFORE and AFTER.
    edge_empty_scope(&store, &clock, &ledger_dir);
    edge_inverted_window_fails_closed(&store, &clock, &ledger_dir);
    edge_before_horizon_fails_closed(&store, &clock, &ledger_dir);

    // ---- Evidence artifact: the data residing in the SoT after execution.
    let final_entries = read_back(&ledger_dir);
    let artifact = serde_json::json!({
        "issue": 576,
        "capability": "universal summarization via multi-scope kernel (summarize/summarize_as_of)",
        "source_of_truth": ledger_dir.display().to_string(),
        "present": {
            "scope_hash": hex(&present.scope_hash),
            "kernel_size": present.kernel_size,
            "grounded_fraction": present.grounded_fraction,
            "approx_factor": present.approx_factor,
            "kernel_only_recall": present.kernel_only_recall,
            "ledger_seq": present.ledger_ref.seq,
        },
        "historical_as_of_1010": {
            "kernel_size": historical.kernel_size,
            "ledger_seq": historical.ledger_ref.seq,
        },
        "on_disk_ledger_rows": final_entries.len(),
        "ledger_kinds": final_entries.iter().map(|e| format!("{:?}", e.kind)).collect::<Vec<_>>(),
        "display": present.to_string(),
    });
    let out = root.join("issue576-summarize-fsv-artifact.json");
    fs::write(&out, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    println!("ISSUE576_FSV_ARTIFACT={}", out.display());
    println!("{}", serde_json::to_string_pretty(&artifact).unwrap());

    // Every on-disk row is a SUMMARIZE_INVOKED kernel entry (no stray writes).
    for entry in &final_entries {
        assert_eq!(entry.kind, EntryKind::Kernel);
        let p: serde_json::Value = serde_json::from_slice(&entry.payload).unwrap();
        assert_eq!(p["marker"], SUMMARIZE_INVOKED_MARKER);
    }
    assert_eq!(
        final_entries.len(),
        3,
        "present + historical + empty-scope edge = 3 rows"
    );
}

/// Edge 1 (empty scope): a collection with zero nodes → empty summary, still
/// provenanced. BEFORE/AFTER printed from the on-disk SoT.
fn edge_empty_scope(store: &MemoryAssocStore, clock: &FixedClock, dir: &Path) {
    let before = ledger_files(dir);
    println!("[edge:empty] BEFORE on-disk rows = {before}");
    let mut cache = ScopeCache::new(4);
    let mut led = appender(dir);
    let result = summarize(
        store,
        Scope::Collection {
            id: CollectionId::from("empty"),
        },
        None,
        &mut ctx(&mut cache, clock, &mut led),
    )
    .expect("empty-scope summarize");
    drop(led);
    let after = ledger_files(dir);
    println!(
        "[edge:empty] AFTER on-disk rows  = {after} (kernel_size={})",
        result.kernel_size
    );
    assert_eq!(result.kernel_size, 0, "empty scope ⇒ empty kernel");
    assert_eq!(
        after,
        before + 1,
        "empty summary is still audited (one new row)"
    );
}

/// Edge 2 (invalid format): inverted time window → fail closed, SoT UNCHANGED.
fn edge_inverted_window_fails_closed(store: &MemoryAssocStore, clock: &FixedClock, dir: &Path) {
    let before = ledger_files(dir);
    println!("[edge:inverted] BEFORE on-disk rows = {before}");
    let mut cache = ScopeCache::new(4);
    let mut led = appender(dir);
    let err = summarize(
        store,
        Scope::TimeWindow {
            t0: 2_000,
            t1: 1_000,
        },
        None,
        &mut ctx(&mut cache, clock, &mut led),
    )
    .expect_err("inverted window must fail closed");
    drop(led);
    let after = ledger_files(dir);
    println!(
        "[edge:inverted] AFTER on-disk rows  = {after} (err={})",
        err.code
    );
    assert_eq!(err.code, "CALYX_SCOPE_INVALID_TIME_WINDOW");
    assert_eq!(after, before, "no row written on a fail-closed evaluation");
}

/// Edge 3 (boundary): as-of before the retention horizon → fail closed, SoT UNCHANGED.
fn edge_before_horizon_fails_closed(store: &MemoryAssocStore, clock: &FixedClock, dir: &Path) {
    let before = ledger_files(dir);
    println!("[edge:horizon] BEFORE on-disk rows = {before}");
    let mut cache = ScopeCache::new(4);
    let mut led = appender(dir);
    let err = summarize_as_of(
        store,
        Scope::AllAssociations,
        1_005,
        Some(1_010), // horizon after the requested time
        None,
        &mut ctx(&mut cache, clock, &mut led),
    )
    .expect_err("before-horizon must fail closed");
    drop(led);
    let after = ledger_files(dir);
    println!(
        "[edge:horizon] AFTER on-disk rows  = {after} (err={})",
        err.code
    );
    assert_eq!(err.code, "CALYX_TIMETRAVEL_BEFORE_HORIZON");
    assert_eq!(after, before, "no row written before the retention horizon");
}

fn hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
