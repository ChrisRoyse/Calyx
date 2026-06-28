use calyx_aster::vault::AsterVault;
use calyx_core::{CxId, FixedClock, LedgerRef, LensId, VaultStore};
use calyx_sextant::query::{AskSpec, GraphHop, UniversalQuery, VectorQuery, execute, plan};
use calyx_sextant::{CALYX_ANSWER_SYNTHESIS_UNAVAILABLE, CALYX_SEXTANT_ASSOC_GRAPH_MISSING};
use serde_json::{Value, json};

use super::hex;

pub(super) fn graph_hop_fail_closed(vault: &AsterVault<FixedClock>, cx_id: CxId) -> Value {
    let before_seq = vault.latest_seq();
    let err = execute(
        vault,
        plan(
            vault,
            &UniversalQuery {
                graph_hop: Some(GraphHop {
                    from_cx_ids: vec![cx_id],
                    hop_kind: "related".to_string(),
                    max_hops: 1,
                }),
                cost_cap_ms: Some(10_000),
                explain: true,
                ..UniversalQuery::default()
            },
        )
        .unwrap(),
    )
    .unwrap_err();
    let after_seq = vault.latest_seq();
    assert_eq!(err.code, CALYX_SEXTANT_ASSOC_GRAPH_MISSING);
    assert_eq!(before_seq, after_seq);
    json!({
        "before_seq": before_seq,
        "after_seq": after_seq,
        "code": err.code,
        "message": err.message
    })
}

pub(super) fn vector_empty_rows(vault: &AsterVault<FixedClock>, lens_id: LensId) -> usize {
    let result = execute(
        vault,
        plan(
            vault,
            &UniversalQuery {
                vector: Some(VectorQuery {
                    lens_ids: vec![lens_id],
                    query_vec: vec![0.1, 0.2],
                    limit: 5,
                }),
                cost_cap_ms: Some(10_000),
                explain: true,
                ..UniversalQuery::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(result.rows.is_empty());
    result.rows.len()
}

pub(super) fn ask_synthesis_fail_closed(
    vault: &AsterVault<FixedClock>,
    cx_id: CxId,
    stored_provenance: LedgerRef,
) -> Value {
    let before_seq = vault.latest_seq();
    let query = UniversalQuery {
        ask: Some(AskSpec {
            question: "test question".to_string(),
            context_cx_ids: vec![cx_id],
            top_k: 1,
            oracle: false,
        }),
        cost_cap_ms: Some(5_000),
        explain: true,
        ..UniversalQuery::default()
    };
    let err = execute(vault, plan(vault, &query).unwrap()).unwrap_err();
    let after_seq = vault.latest_seq();
    let after_stored = vault.get(cx_id, after_seq).unwrap();
    assert_eq!(err.code, CALYX_ANSWER_SYNTHESIS_UNAVAILABLE);
    assert_eq!(before_seq, after_seq);
    assert_eq!(after_stored.provenance, stored_provenance);
    json!({
        "cx_id": cx_id.to_string(),
        "before_seq": before_seq,
        "after_seq": after_seq,
        "error_code": err.code,
        "message": err.message,
        "stored_ledger_ref_seq": stored_provenance.seq,
        "stored_ledger_ref_hash": hex(&stored_provenance.hash)
    })
}
