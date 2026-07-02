//! Freshness regression tests for the nav-extension runtime (issue #1104):
//! seq-domain discipline between ledger-ref seqs and vault commit seqs.

use calyx_core::VaultStore;
use serde_json::json;

use super::extension_tests::{TestEnv, call_ok, maybe_write_fsv_json, populated_vault, server};

#[test]
fn define_and_search_skill_stay_fresh_after_replay_only_ingest() {
    // Replay-only ingest appends an idempotency-ledger retry entry: the vault
    // commit seq advances while Base docs stay unchanged (same content-neutral
    // commit class as #1100). The nav engine is built from docs pinned at the
    // load snapshot, so FreshDerived must keep passing afterwards; before the
    // issue #1104 fix this failed with a spurious CALYX_STALE_DERIVED because
    // ledger-ref seqs were compared against vault commit seqs.
    let _env = TestEnv::new("replay-fresh");
    let server = server();
    let ingested = populated_vault(&server, "v");
    let seeded_cx = ingested[0]["cx_id"].as_str().unwrap();

    let replay = call_ok(
        &server,
        40,
        "calyx.ingest",
        json!({"vault": "v", "input": "alpha alpha"}),
    );
    assert_eq!(replay["new"], json!(false), "expected idempotent replay");
    assert_eq!(replay["cx_id"].as_str().unwrap(), seeded_cx);

    let panel = call_ok(&server, 41, "calyx.list_panel", json!({"vault": "v"}));
    let lens = panel["slots"]
        .as_array()
        .expect("panel slots")
        .iter()
        .find(|slot| slot["state"] == "active" && slot["name"] == "byte_axis")
        .and_then(|slot| slot["slot"].as_u64())
        .expect("active byte_axis slot");

    let defined = call_ok(
        &server,
        42,
        "calyx.define",
        json!({"vault": "v", "lens": lens, "index": 0}),
    );
    assert_eq!(defined["definition"]["cx_id"].as_str().unwrap().len(), 32);
    assert!(
        !defined["definition"]["slots"]
            .as_array()
            .unwrap()
            .is_empty(),
        "definition must carry gathered slots"
    );

    let hits = call_ok(
        &server,
        43,
        "calyx.search_skill",
        json!({"vault": "v", "skill": "skill-root", "query": "alpha"}),
    );
    assert!(!hits["hits"].as_array().unwrap().is_empty());

    // Source-of-truth readback: open the vault directly and confirm the seq
    // drift that used to trip the freshness gate is really present — the
    // vault commit seq moved past every per-doc ledger-ref seq, yet the nav
    // tools above stayed fresh because they reason in the commit-seq domain
    // of one pinned snapshot.
    let home = crate::tools::vault::store::home_dir().unwrap();
    let resolved = crate::tools::vault::store::resolve_vault_info(&home, "v").unwrap();
    let vault = calyx_aster::vault::AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        crate::tools::vault::store::vault_salt(resolved.vault_id, &resolved.name),
        calyx_aster::vault::VaultOptions::default(),
    )
    .unwrap();
    let latest_seq = vault.latest_seq();
    let snapshot = vault.snapshot();
    let mut doc_count = 0usize;
    let mut max_provenance_seq = 0u64;
    for (key, _) in vault
        .scan_cf_at(snapshot, calyx_aster::cf::ColumnFamily::Base)
        .unwrap()
    {
        let bytes: [u8; 16] = key.as_slice().try_into().unwrap();
        let cx = vault
            .get(calyx_core::CxId::from_bytes(bytes), snapshot)
            .unwrap();
        doc_count += 1;
        max_provenance_seq = max_provenance_seq.max(cx.provenance.seq);
    }
    assert_eq!(doc_count, 5, "replay must not add Base docs");
    assert!(
        latest_seq > max_provenance_seq,
        "scenario must exhibit the adversarial drift (latest_seq {latest_seq} \
         <= max provenance seq {max_provenance_seq})"
    );

    maybe_write_fsv_json(
        "mcp-search-extensions-replay-freshness.json",
        &json!({
            "source_of_truth": "JSON-RPC results from calyx.define and calyx.search_skill after a replay-only ingest commit, plus direct vault readback of seq domains",
            "replay_ingest": replay,
            "vault_readback": {
                "latest_seq": latest_seq,
                "max_doc_provenance_seq": max_provenance_seq,
                "base_doc_count": doc_count,
            },
            "define": {
                "lens": lens,
                "cx_id": defined["definition"]["cx_id"],
                "slot_count": defined["definition"]["slots"].as_array().unwrap().len(),
            },
            "search_skill_hit_count": hits["hits"].as_array().unwrap().len(),
        }),
    );
}
