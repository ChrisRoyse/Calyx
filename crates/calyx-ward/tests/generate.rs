#[path = "generate/support.rs"]
mod support;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use calyx_core::FixedClock;
use calyx_ledger::{
    DirectoryLedgerStore, LedgerAppender, LedgerCfStore, MemoryLedgerStore, decode,
};
use calyx_ward::{
    GUARDED_REJECT_TAG, GUARDED_REJECT_UNPROVENANCED_TAG, GenerateOutput, GuardPolicy,
    NoveltyAction, NoveltyHandler, NoveltyStatus, VaultSink, WardError, guard_generate,
    guard_generate_with_ledger,
};
use proptest::prelude::*;
use serde_json::{Value, json};
use support::{
    FailingSink, FileSink, MemorySink, MockLens, STYLE_SLOT, cos_vector, empty_profile,
    generate_input, handler_for, identity_profile, write_json, write_manifest,
};

#[test]
fn in_region_generation_is_accepted_with_guarded_pass_tag() {
    let sink = MemorySink::default();
    let output = guard_generate(
        &identity_profile(NoveltyAction::NewRegion, true),
        &generate_input(true, true),
        &MockLens::audio(cos_vector(0.85)),
        &MockLens::text(cos_vector(0.86)),
        &handler_for(sink.clone()),
        true,
    )
    .unwrap();

    match output {
        GenerateOutput::Accepted {
            verdict,
            provenance_tag,
            ledger_ref,
        } => {
            assert!(verdict.overall_pass);
            assert_eq!(provenance_tag, "guarded:pass");
            assert_eq!(ledger_ref, None);
        }
        other => panic!("expected accepted output, got {other:?}"),
    }
    assert!(sink.novel_records().unwrap().is_empty());
}

#[test]
fn accepted_generation_can_append_guard_ledger_row() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(27_200)).unwrap();
    let output = guard_generate_with_ledger(
        &mut appender,
        &identity_profile(NoveltyAction::NewRegion, true),
        &generate_input(true, true),
        &MockLens::audio(cos_vector(0.90)),
        &MockLens::text(cos_vector(0.91)),
        &handler_for(MemorySink::default()),
        true,
    )
    .unwrap();
    let rows = appender.store().scan().unwrap();

    match output {
        GenerateOutput::Accepted { ledger_ref, .. } => {
            assert_eq!(ledger_ref.as_ref().map(|value| value.seq), Some(0));
        }
        other => panic!("expected accepted output, got {other:?}"),
    }
    assert_eq!(rows.len(), 1);
    let entry = decode(&rows[0].bytes).unwrap();
    let payload: Value = serde_json::from_slice(&entry.payload).unwrap();
    assert_eq!(payload["ward_provenance"], "ward_guard_verdict_v1");
    assert_eq!(payload["overall_pass"], true);
}

#[test]
fn rejected_generation_can_append_guard_ledger_row() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(27_201)).unwrap();
    let output = guard_generate_with_ledger(
        &mut appender,
        &identity_profile(NoveltyAction::RejectClosed, true),
        &generate_input(true, true),
        &MockLens::audio(cos_vector(0.90)),
        &MockLens::text(cos_vector(0.20)),
        &handler_for(MemorySink::default()),
        true,
    )
    .unwrap();
    let rows = appender.store().scan().unwrap();

    match output {
        GenerateOutput::Rejected {
            ledger_ref,
            provenance_tag,
            ..
        } => {
            assert_eq!(provenance_tag, GUARDED_REJECT_TAG);
            assert_eq!(ledger_ref.as_ref().map(|value| value.seq), Some(0));
        }
        other => panic!("expected rejected output, got {other:?}"),
    }
    assert_eq!(rows.len(), 1);
    let entry = decode(&rows[0].bytes).unwrap();
    let payload: Value = serde_json::from_slice(&entry.payload).unwrap();
    assert_eq!(payload["ward_provenance"], "ward_guard_verdict_v1");
    assert_eq!(payload["overall_pass"], false);
    assert_eq!(payload["action"], "reject_closed");
}

#[test]
fn out_of_region_new_region_routes_to_novelty_record() {
    let sink = MemorySink::default();
    let output = guard_generate(
        &identity_profile(NoveltyAction::NewRegion, true),
        &generate_input(true, true),
        &MockLens::audio(cos_vector(0.85)),
        &MockLens::text(cos_vector(0.40)),
        &handler_for(sink.clone()),
        true,
    )
    .unwrap();

    match output {
        GenerateOutput::Novel { record } => {
            assert_eq!(record.status, NoveltyStatus::AwaitingGrounding);
            assert_eq!(record.failing_verdicts[0].slot, STYLE_SLOT);
        }
        other => panic!("expected novel output, got {other:?}"),
    }
    assert_eq!(sink.novel_records().unwrap().len(), 1);
}

#[test]
fn reject_closed_preserves_rejected_verdict_after_sink_write() {
    let sink = MemorySink::default();
    let output = guard_generate(
        &identity_profile(NoveltyAction::RejectClosed, true),
        &generate_input(true, true),
        &MockLens::audio(cos_vector(0.90)),
        &MockLens::text(cos_vector(0.20)),
        &handler_for(sink.clone()),
        true,
    )
    .unwrap();

    match output {
        GenerateOutput::Rejected {
            verdict,
            provenance_tag,
            ledger_ref,
        } => {
            assert!(!verdict.overall_pass);
            assert_eq!(verdict.failing_slots()[0].slot, STYLE_SLOT);
            assert_eq!(provenance_tag, GUARDED_REJECT_UNPROVENANCED_TAG);
            assert_eq!(ledger_ref, None);
        }
        other => panic!("expected rejected output, got {other:?}"),
    }
    assert_eq!(
        sink.novel_records().unwrap()[0].status,
        NoveltyStatus::Rejected
    );
}

#[test]
fn high_stakes_uncalibrated_profile_fails_before_lens_execution() {
    let speaker = MockLens::audio(cos_vector(0.90));
    let style = MockLens::text(cos_vector(0.90));
    let error = guard_generate(
        &identity_profile(NoveltyAction::NewRegion, false),
        &generate_input(true, true),
        &speaker,
        &style,
        &handler_for(MemorySink::default()),
        true,
    )
    .unwrap_err();

    assert!(matches!(error, WardError::Provisional { .. }));
    assert_eq!(speaker.calls(), 0);
    assert_eq!(style.calls(), 0);
}

#[test]
fn inert_kofn_identity_profile_fails_before_lens_execution() {
    let mut profile = identity_profile(NoveltyAction::NewRegion, true);
    profile.guard_profile.policy = GuardPolicy::KofN { k: 0 };
    let speaker = MockLens::audio(cos_vector(0.90));
    let style = MockLens::text(cos_vector(0.90));
    let error = guard_generate(
        &profile,
        &generate_input(true, true),
        &speaker,
        &style,
        &handler_for(MemorySink::default()),
        true,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        WardError::InertProfile {
            reason: "kofn_zero",
            ..
        }
    ));
    assert_eq!(speaker.calls(), 0);
    assert_eq!(style.calls(), 0);
}

#[test]
fn empty_identity_profile_fails_before_lens_execution() {
    let speaker = MockLens::audio(cos_vector(0.90));
    let style = MockLens::text(cos_vector(0.90));
    let error = guard_generate(
        &empty_profile(),
        &generate_input(true, true),
        &speaker,
        &style,
        &handler_for(MemorySink::default()),
        false,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        WardError::InvalidRequiredSlotDerivation { .. }
    ));
    assert_eq!(speaker.calls(), 0);
    assert_eq!(style.calls(), 0);
}

#[test]
fn missing_required_speaker_input_returns_missing_slot() {
    let error = guard_generate(
        &identity_profile(NoveltyAction::NewRegion, true),
        &generate_input(false, true),
        &MockLens::audio(cos_vector(0.90)),
        &MockLens::text(cos_vector(0.90)),
        &handler_for(MemorySink::default()),
        true,
    )
    .unwrap_err();

    assert_eq!(error.code(), "CALYX_GUARD_MISSING_SLOT");
}

#[test]
fn novelty_sink_failure_propagates_without_accepting() {
    let error = guard_generate(
        &identity_profile(NoveltyAction::NewRegion, true),
        &generate_input(true, true),
        &MockLens::audio(cos_vector(0.90)),
        &MockLens::text(cos_vector(0.20)),
        &NoveltyHandler::new(Arc::new(FailingSink), Arc::new(FixedClock::new(1))),
        true,
    )
    .unwrap_err();

    assert_eq!(error.code(), "CALYX_GUARD_NOVELTY_SINK");
}

proptest! {
    #[test]
    fn proptest_acceptance_matches_per_slot_threshold(
        speaker_cos in 0.0f32..1.0,
        style_cos in 0.0f32..1.0,
    ) {
        let output = guard_generate(
            &identity_profile(NoveltyAction::NewRegion, true),
            &generate_input(true, true),
            &MockLens::audio(cos_vector(speaker_cos)),
            &MockLens::text(cos_vector(style_cos)),
            &handler_for(MemorySink::default()),
            true,
        ).unwrap();
        let accepted = matches!(output, GenerateOutput::Accepted { .. });
        prop_assert_eq!(accepted, speaker_cos >= 0.70 && style_cos >= 0.70);
    }
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_GENERATE_FSV_DIR"]
fn issue272_guard_generate_fsv_writes_readbacks() {
    let root = PathBuf::from(
        std::env::var("CALYX_WARD_GENERATE_FSV_DIR")
            .expect("CALYX_WARD_GENERATE_FSV_DIR is required"),
    );
    fs::create_dir_all(&root).unwrap();
    let ledger_dir = root.join("ledger-cf");
    fs::create_dir_all(&ledger_dir).unwrap();
    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap();
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).unwrap(),
        FixedClock::new(27_200),
    )
    .unwrap();

    let accepted = guard_generate_with_ledger(
        &mut appender,
        &identity_profile(NoveltyAction::NewRegion, true),
        &generate_input(true, true),
        &MockLens::audio(cos_vector(0.88)),
        &MockLens::text(cos_vector(0.89)),
        &handler_for(MemorySink::default()),
        true,
    )
    .unwrap();
    let novel_sink = FileSink::new(root.join("novelty-new-region"));
    let novel = guard_generate(
        &identity_profile(NoveltyAction::NewRegion, true),
        &generate_input(true, true),
        &MockLens::audio(cos_vector(0.91)),
        &MockLens::text(cos_vector(0.38)),
        &handler_for(novel_sink.clone()),
        true,
    )
    .unwrap();
    let reject_sink = FileSink::new(root.join("novelty-reject"));
    let rejected = guard_generate(
        &identity_profile(NoveltyAction::RejectClosed, true),
        &generate_input(true, true),
        &MockLens::audio(cos_vector(0.91)),
        &MockLens::text(cos_vector(0.30)),
        &handler_for(reject_sink.clone()),
        true,
    )
    .unwrap();
    let rejected_ledger = guard_generate_with_ledger(
        &mut appender,
        &identity_profile(NoveltyAction::RejectClosed, true),
        &generate_input(true, true),
        &MockLens::audio(cos_vector(0.91)),
        &MockLens::text(cos_vector(0.29)),
        &handler_for(FileSink::new(root.join("novelty-reject-ledger"))),
        true,
    )
    .unwrap();
    let after_rows = appender.store().scan().unwrap();
    let entries = after_rows
        .iter()
        .map(|row| decode(&row.bytes).unwrap())
        .collect::<Vec<_>>();
    let provisional = guard_generate(
        &identity_profile(NoveltyAction::NewRegion, false),
        &generate_input(true, true),
        &MockLens::audio(cos_vector(0.91)),
        &MockLens::text(cos_vector(0.91)),
        &handler_for(MemorySink::default()),
        true,
    )
    .unwrap_err();

    let files = [
        write_json(&root, "accepted-output.json", &json!(accepted)),
        write_json(
            &root,
            "ledger-readback.json",
            &json!({
                "ledger_dir": ledger_dir,
                "before_rows": support::row_readback(&before_rows),
                "after_rows": support::row_readback(&after_rows),
                "entries": support::entry_readback(&entries),
            }),
        ),
        write_json(&root, "novel-output.json", &json!(novel)),
        write_json(
            &root,
            "novelty-new-region-readback.json",
            &json!(novel_sink.novel_records().unwrap()),
        ),
        write_json(&root, "rejected-output.json", &json!(rejected)),
        write_json(
            &root,
            "rejected-ledger-output.json",
            &json!(rejected_ledger),
        ),
        write_json(
            &root,
            "novelty-reject-readback.json",
            &json!(reject_sink.novel_records().unwrap()),
        ),
        write_json(
            &root,
            "provisional-error.json",
            &json!({"code": provisional.code(), "message": provisional.to_string()}),
        ),
    ];
    write_manifest(&root, &files);
}
