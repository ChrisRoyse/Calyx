use calyx_core::{CalyxWarning, CxId, FixedClock, LensId, SlotId};
use calyx_ledger::{
    ActorId, AuditFilter, EntryKind, FusionMode, FusionWeights, LedgerAppender, LedgerCfStore,
    MemoryLedgerStore, QuarantineSet, SlotWeight, SubjectId, audit, get_answer_trace,
    get_provenance,
};
use serde_json::json;

#[test]
fn get_provenance_returns_only_entries_for_cx() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(100)).unwrap();
    append_json(
        &mut appender,
        EntryKind::Ingest,
        SubjectId::Cx(cx(1)),
        json!({"cx_id": cx(1).to_string()}),
    );
    append_json(
        &mut appender,
        EntryKind::Ingest,
        SubjectId::Cx(cx(2)),
        json!({"cx_id": cx(2).to_string()}),
    );
    append_json(
        &mut appender,
        EntryKind::Measure,
        SubjectId::Cx(cx(3)),
        json!({"cx_id": cx(3).to_string()}),
    );
    let store = appender.into_store();

    let quarantine = QuarantineSet::default();
    let found = get_provenance(&store, &quarantine, cx(1)).unwrap();
    let missing = get_provenance(&store, &quarantine, cx(9)).unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].seq, 0);
    assert!(missing.is_empty());
}

#[test]
fn get_answer_trace_decodes_complete_path_and_fusion_weights() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(200)).unwrap();
    let answer_id = b"answer-trace".to_vec();
    let fusion = fusion_weights();
    let kernel_id = cx(88).as_bytes().to_vec();
    let guard_id = b"guard-audit".to_vec();
    append_json(
        &mut appender,
        EntryKind::Kernel,
        SubjectId::Kernel(kernel_id.clone()),
        json!({"kernel_id": cx(88).to_string(), "recall_ratio": 0.99}),
    );
    append_json(
        &mut appender,
        EntryKind::Guard,
        SubjectId::Guard(guard_id.clone()),
        json!({"guard_id": "guard-audit", "pass": true, "tau": 0.8}),
    );
    append_json(
        &mut appender,
        EntryKind::Answer,
        SubjectId::Query(answer_id.clone()),
        json!({
            "complete": true,
            "expected_hops": 2,
            "kernel_id": cx(88).to_string(),
            "guard_id": "guard-audit",
            "path": [
                {"from_id": cx(10).to_string(), "cx_id": cx(11).to_string(), "hop": 0, "score": 0.9, "lens_id": lens(1).to_string(), "ledger_ref": {"seq": 42}},
                {"from_id": cx(11).to_string(), "cx_id": cx(12).to_string(), "hop": 1, "score": 0.7, "lens_id": lens(2).to_string(), "ledger_seq": 43}
            ],
            "fusion_weights": fusion,
            "guard_result": {"pass": true},
            "freshness_ts": 777
        }),
    );
    let store = appender.into_store();

    let trace = get_answer_trace(&store, &QuarantineSet::default(), &answer_id).unwrap();

    assert!(trace.is_trusted());
    assert_eq!(trace.path.len(), 2);
    assert_eq!(trace.path[0].cx_id, cx(11));
    assert_eq!(trace.path[0].ledger_seq, 42);
    assert_eq!(trace.path[1].ledger_seq, 43);
    assert_eq!(trace.path[1].lens_id, Some(lens(2)));
    assert_eq!(trace.kernel_entry.as_ref().unwrap().seq, 0);
    assert_eq!(trace.guard_entry.as_ref().unwrap().seq, 1);
    assert_eq!(trace.fusion_weights, Some(fusion));
    assert_eq!(
        trace.guard_result,
        Some(json!({"guard_id": "guard-audit", "pass": true, "tau": 0.8}))
    );
    assert_eq!(trace.freshness_ts, Some(777));
}

#[test]
fn answer_path_without_complete_marker_is_unprovenanced() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(250)).unwrap();
    let answer_id = b"unmarked-answer".to_vec();
    append_json(
        &mut appender,
        EntryKind::Answer,
        SubjectId::Query(answer_id.clone()),
        json!({
            "path": [
                {"from_id": cx(1).to_string(), "cx_id": cx(2).to_string(), "hop": 0, "score": 0.5}
            ],
            "fusion_weights": fusion_weights()
        }),
    );
    let store = appender.into_store();

    let trace = get_answer_trace(&store, &QuarantineSet::default(), &answer_id).unwrap();

    assert_eq!(trace.path.len(), 1);
    assert!(!trace.complete);
    assert!(!trace.is_trusted());
    assert_eq!(
        trace.warnings,
        vec![CalyxWarning::unprovenanced(
            "answer_trace.partial_or_unmarked"
        )]
    );
}

#[test]
fn audit_filters_by_kind_and_time_range() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(300)).unwrap();
    for index in 0..10 {
        let kind = if index % 2 == 0 {
            EntryKind::Ingest
        } else {
            EntryKind::Measure
        };
        append_json(
            &mut appender,
            kind,
            SubjectId::Cx(cx(index as u8)),
            json!({"cx_id": cx(index as u8).to_string()}),
        );
    }
    let store = appender.into_store();

    let ingest = audit(
        &store,
        &QuarantineSet::default(),
        AuditFilter {
            kind: Some(EntryKind::Ingest),
            ..AuditFilter::default()
        },
    )
    .unwrap();
    let empty = audit(
        &store,
        &QuarantineSet::default(),
        AuditFilter {
            ts_range: Some((10_000, 10_001)),
            ..AuditFilter::default()
        },
    )
    .unwrap();

    assert_eq!(ingest.len(), 5);
    assert!(ingest.iter().all(|entry| entry.kind == EntryKind::Ingest));
    assert!(empty.is_empty());
}

#[test]
fn audit_filter_skips_unmatched_quarantined_rows_but_fails_for_matching_rows() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(350)).unwrap();
    append_json(
        &mut appender,
        EntryKind::Ingest,
        SubjectId::Cx(cx(1)),
        json!({"cx_id": cx(1).to_string()}),
    );
    append_json(
        &mut appender,
        EntryKind::Measure,
        SubjectId::Cx(cx(2)),
        json!({"cx_id": cx(2).to_string()}),
    );
    append_json(
        &mut appender,
        EntryKind::Ingest,
        SubjectId::Cx(cx(3)),
        json!({"cx_id": cx(3).to_string()}),
    );
    let store = appender.into_store();
    let quarantine = QuarantineSet::from_ranges(std::iter::once(1..2)).unwrap();

    let ingest = audit(
        &store,
        &quarantine,
        AuditFilter {
            kind: Some(EntryKind::Ingest),
            ..AuditFilter::default()
        },
    )
    .unwrap();
    let matching_error = audit(
        &store,
        &quarantine,
        AuditFilter {
            kind: Some(EntryKind::Measure),
            ..AuditFilter::default()
        },
    )
    .unwrap_err();
    let range_error = audit(
        &store,
        &quarantine,
        AuditFilter {
            seq_range: Some((1, 2)),
            ..AuditFilter::default()
        },
    )
    .unwrap_err();

    assert_eq!(
        ingest.iter().map(|entry| entry.seq).collect::<Vec<_>>(),
        vec![0, 2]
    );
    assert_eq!(matching_error.code, "CALYX_LEDGER_CHAIN_BROKEN");
    assert_eq!(range_error.code, "CALYX_LEDGER_CHAIN_BROKEN");
}

#[test]
fn audit_rejects_mismatched_physical_row_key_even_when_embedded_seq_is_clean() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(360)).unwrap();
    append_json(
        &mut appender,
        EntryKind::Ingest,
        SubjectId::Cx(cx(1)),
        json!({"cx_id": cx(1).to_string()}),
    );
    let row = appender.into_store().scan().unwrap().remove(0);
    let mut store = MemoryLedgerStore::default();
    store.insert_raw(9, row.bytes);
    let quarantine = QuarantineSet::from_ranges(std::iter::once(9..10)).unwrap();

    let error = audit(
        &store,
        &quarantine,
        AuditFilter {
            kind: Some(EntryKind::Ingest),
            ..AuditFilter::default()
        },
    )
    .unwrap_err();

    assert_eq!(error.code, "CALYX_LEDGER_CHAIN_BROKEN");
    assert!(
        error
            .message
            .contains("ledger row key 9 does not match encoded seq 0")
    );
}

#[test]
fn provenance_ignores_untyped_payload_strings_but_keeps_explicit_cx_fields() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(375)).unwrap();
    append_json(
        &mut appender,
        EntryKind::Measure,
        SubjectId::Lens(lens(9)),
        json!({
            "comment": cx(1).to_string(),
            "nested": {"note": cx(1).to_string()},
            "array": [cx(1).to_string()]
        }),
    );
    append_json(
        &mut appender,
        EntryKind::Answer,
        SubjectId::Query(b"answer-with-path".to_vec()),
        json!({"path": [{"from_id": cx(1).to_string(), "to_id": cx(2).to_string()}]}),
    );
    append_json(
        &mut appender,
        EntryKind::Guard,
        SubjectId::Cx(cx(1)),
        json!({"comment": "subject match is typed"}),
    );
    let store = appender.into_store();

    let found = get_provenance(&store, &QuarantineSet::default(), cx(1)).unwrap();

    assert_eq!(
        found.iter().map(|entry| entry.seq).collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn quarantine_is_fail_closed_for_all_query_surfaces() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(400)).unwrap();
    let answer_id = b"quarantined-answer".to_vec();
    append_json(
        &mut appender,
        EntryKind::Ingest,
        SubjectId::Cx(cx(1)),
        json!({"cx_id": cx(1).to_string()}),
    );
    append_json(
        &mut appender,
        EntryKind::Answer,
        SubjectId::Query(answer_id.clone()),
        json!({"complete": true, "path": [], "fusion_weights": fusion_weights()}),
    );
    let store = appender.into_store();
    let quarantine_answer = QuarantineSet::from_ranges(std::iter::once(1..2)).unwrap();
    let quarantine_ingest = QuarantineSet::from_ranges(std::iter::once(0..1)).unwrap();

    let trace_error = get_answer_trace(&store, &quarantine_answer, &answer_id).unwrap_err();
    let audit_error = audit(
        &store,
        &quarantine_answer,
        AuditFilter {
            seq_range: Some((1, 2)),
            ..AuditFilter::default()
        },
    )
    .unwrap_err();
    let provenance_error = get_provenance(&store, &quarantine_ingest, cx(1)).unwrap_err();

    assert_eq!(trace_error.code, "CALYX_LEDGER_CHAIN_BROKEN");
    assert_eq!(audit_error.code, "CALYX_LEDGER_CHAIN_BROKEN");
    assert_eq!(provenance_error.code, "CALYX_LEDGER_CHAIN_BROKEN");
}

#[test]
fn quarantine_is_checked_before_decoding_row_bytes() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(450)).unwrap();
    append_json(
        &mut appender,
        EntryKind::Ingest,
        SubjectId::Cx(cx(1)),
        json!({"cx_id": cx(1).to_string()}),
    );
    append_json(
        &mut appender,
        EntryKind::Ingest,
        SubjectId::Cx(cx(2)),
        json!({"cx_id": cx(2).to_string()}),
    );
    let mut store = appender.into_store();
    let mut poisoned = store
        .scan()
        .unwrap()
        .into_iter()
        .find(|row| row.seq == 1)
        .unwrap();
    poisoned.bytes[8] ^= 1;
    store.insert_raw(1, poisoned.bytes);

    let quarantine = QuarantineSet::from_ranges(std::iter::once(1..2)).unwrap();
    let error = get_provenance(&store, &quarantine, cx(1)).unwrap_err();

    assert_eq!(error.code, "CALYX_LEDGER_CHAIN_BROKEN");
}

#[test]
fn partial_answer_hop_rows_are_unprovenanced_not_trusted() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(500)).unwrap();
    let answer_id = cx(40).as_bytes().to_vec();
    append_json(
        &mut appender,
        EntryKind::Answer,
        SubjectId::Query(answer_id.clone()),
        json!({
            "query_id": cx(40).to_string(),
            "anchor_kernel_node_id": cx(41).to_string(),
            "from_id": cx(10).to_string(),
            "to_id": cx(11).to_string(),
            "hop_index": 0,
            "hop_score": 0.8
        }),
    );
    append_json(
        &mut appender,
        EntryKind::Answer,
        SubjectId::Query(answer_id.clone()),
        json!({
            "query_id": cx(40).to_string(),
            "anchor_kernel_node_id": cx(41).to_string(),
            "from_id": cx(11).to_string(),
            "to_id": cx(12).to_string(),
            "hop_index": 1,
            "hop_score": 0.6
        }),
    );
    let store = appender.into_store();

    let trace = get_answer_trace(&store, &QuarantineSet::default(), &answer_id).unwrap();
    let query_provenance = get_provenance(&store, &QuarantineSet::default(), cx(40)).unwrap();
    let anchor_provenance = get_provenance(&store, &QuarantineSet::default(), cx(41)).unwrap();

    assert_eq!(trace.path.len(), 2);
    assert_eq!(
        query_provenance
            .iter()
            .map(|entry| entry.seq)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(
        anchor_provenance
            .iter()
            .map(|entry| entry.seq)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert!(!trace.complete);
    assert!(!trace.is_trusted());
    assert_eq!(
        trace.warnings,
        vec![CalyxWarning::unprovenanced(
            "answer_trace.partial_or_unmarked"
        )]
    );
}

fn append_json<S, C>(
    appender: &mut LedgerAppender<S, C>,
    kind: EntryKind,
    subject: SubjectId,
    value: serde_json::Value,
) where
    S: LedgerCfStore,
    C: calyx_core::Clock,
{
    appender
        .append(
            kind,
            subject,
            serde_json::to_vec(&value).unwrap(),
            ActorId::Service("audit-test".to_string()),
        )
        .unwrap();
}

fn fusion_weights() -> FusionWeights {
    FusionWeights {
        mode: FusionMode::WeightedRrf,
        k: 2,
        candidates: vec![cx(1), cx(2)],
        weights: vec![SlotWeight {
            slot_id: SlotId::new(0),
            weight: 1.0,
        }],
        single_slot: None,
    }
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn lens(seed: u8) -> LensId {
    LensId::from_bytes([seed; 16])
}
