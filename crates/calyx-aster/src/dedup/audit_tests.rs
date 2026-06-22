use std::collections::BTreeMap;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Clock, CxId, FixedClock, Modality, SlotId, SlotVector,
    VaultId, VaultStore,
};
use calyx_ledger::{
    ActorId, EntryKind, LedgerAppender, LedgerCfStore, MemoryLedgerStore, SubjectId,
};
use ulid::Ulid;

use super::*;
use crate::dedup::{
    DedupAction, DedupPolicy, DedupResult, IngestInput, TauStrategy, TctCosineConfig, ingest_at,
};
use crate::recurrence::read_series;
use crate::vault::encode::encode_constellation_base;
use crate::vault::{AsterVault, VaultOptions};

#[test]
fn audit_without_merges_returns_empty_history() {
    let root = test_root("dedup-audit-empty");
    let vault = durable_vault_with_policy(&root, DedupPolicy::Off);
    let first = ingest_at(
        &vault,
        &input("audit-empty", [1.0, 0.0], [1.0, 0.0]),
        EpochSecs(100),
        None,
    )
    .expect("ingest");
    let id = new_id(first);

    let report = dedup_audit(&vault, id).expect("audit");

    assert_eq!(report.cx_id, id);
    assert!(report.merges.is_empty());
    assert!(report.undo_entries.is_empty());
    assert!(report.occurrences.is_empty());
    assert_eq!(report.reversal_token.snapshot_cx_ids, vec![id]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn undo_rejects_token_from_wrong_vault() {
    let root = test_root("dedup-audit-wrong-vault");
    let vault = durable_vault(&root);
    let first = ingest_at(
        &vault,
        &input("audit-wrong-vault", [1.0, 0.0], [1.0, 0.0]),
        EpochSecs(100),
        None,
    )
    .expect("ingest");
    let mut token = dedup_audit(&vault, new_id(first))
        .expect("audit")
        .reversal_token;
    token.vault_id = other_vault_id();

    let error = dedup_undo(&vault, &token).expect_err("wrong vault rejected");

    assert_eq!(error.code, CALYX_DEDUP_WRONG_VAULT);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn audit_and_undo_recurrence_merges_restore_candidates() {
    let root = test_root("dedup-audit-undo");
    let vault = durable_vault(&root);
    let first = ingest_at(
        &vault,
        &input("audit-a", [1.0, 0.0], [1.0, 0.0]),
        EpochSecs(100),
        None,
    )
    .expect("first");
    let second = ingest_at(
        &vault,
        &input("audit-b", [1.0, 0.0], [0.0, 1.0]),
        EpochSecs(200),
        None,
    )
    .expect("second");
    let third = ingest_at(
        &vault,
        &input("audit-c", [1.0, 0.0], [-1.0, 0.0]),
        EpochSecs(300),
        None,
    )
    .expect("third");
    let into = new_id(first);
    assert_eq!(merge_occurrence(second), 1);
    assert_eq!(merge_occurrence(third), 2);

    let report = dedup_audit(&vault, into).expect("audit");

    assert_eq!(report.merges.len(), 2);
    assert_eq!(report.occurrences.len(), 3);
    assert!(report.merges.iter().all(|merge| merge.recurrence_signature));
    assert_eq!(report.merges[0].per_slot_cos, vec![(slot(0), 1.0)]);
    assert_eq!(report.merges[1].per_slot_cos, vec![(slot(0), 1.0)]);
    assert_eq!(report.reversal_token.ledger_seq_start, 1);
    assert_eq!(report.reversal_token.ledger_seq_end, 2);
    assert_eq!(report.reversal_token.snapshot_cx_ids.len(), 3);
    assert_eq!(report.reversal_token.target_cx_id, into);
    let expected_base = restore_snapshot_base_bytes(&vault, into);

    let restored = dedup_undo(&vault, &report.reversal_token).expect("undo");
    let restored_again = dedup_undo(&vault, &report.reversal_token).expect("undo again");
    let audit_after_undo = dedup_audit(&vault, into).expect("audit after undo");
    let series = read_series(&vault, into).expect("series after undo");

    assert_eq!(restored.len(), 2);
    assert_eq!(restored_again, restored);
    assert_eq!(audit_after_undo.undo_entries.len(), 1);
    assert_eq!(audit_after_undo.undo_entries[0].restored, restored);
    assert!(series.occurrences.is_empty());
    assert_eq!(series.frequency, 0);
    for id in &restored {
        let restored_cx = vault.get(*id, vault.snapshot()).expect("restored");
        assert_eq!(restored_cx.cx_id, *id);
        assert_eq!(
            encode_constellation_base(&restored_cx).expect("restored base bytes"),
            expected_base[id]
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn undo_token_skips_unrelated_interleaved_merge_in_seq_range() {
    let root = test_root("dedup-audit-interleaved");
    let vault = durable_vault(&root);
    let first = ingest_at(
        &vault,
        &input("target-a", [1.0, 0.0], [1.0, 0.0]),
        EpochSecs(100),
        None,
    )
    .expect("target first");
    ingest_at(
        &vault,
        &input("target-b", [1.0, 0.0], [0.0, 1.0]),
        EpochSecs(200),
        None,
    )
    .expect("target second");
    let other = ingest_at(
        &vault,
        &input("other-a", [0.0, 1.0], [1.0, 0.0]),
        EpochSecs(250),
        None,
    )
    .expect("other first");
    ingest_at(
        &vault,
        &input("other-b", [0.0, 1.0], [0.0, 1.0]),
        EpochSecs(260),
        None,
    )
    .expect("other second");
    ingest_at(
        &vault,
        &input("target-c", [1.0, 0.0], [-1.0, 0.0]),
        EpochSecs(300),
        None,
    )
    .expect("target third");
    let target = new_id(first);
    let other = new_id(other);
    let other_merged = dedup_audit(&vault, other).expect("other audit").merges[0].merged_from;
    let report = dedup_audit(&vault, target).expect("target audit");
    assert_eq!(report.reversal_token.ledger_seq_start, 1);
    assert_eq!(report.reversal_token.ledger_seq_end, 4);

    let restored = dedup_undo(&vault, &report.reversal_token).expect("target undo");

    assert_eq!(restored.len(), 2);
    assert!(!restored.contains(&other_merged));
    assert_eq!(
        read_series(&vault, target)
            .expect("target series")
            .frequency,
        0
    );
    assert_eq!(
        read_series(&vault, other).expect("other series").frequency,
        2
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn audit_reports_anchor_conflict_blocks() {
    let root = test_root("dedup-audit-anchor-conflict");
    let vault = durable_vault(&root);
    let first = ingest_at(
        &vault,
        &input("anchor-a", [1.0, 0.0], [1.0, 0.0]).with_anchor(speaker("alice")),
        EpochSecs(100),
        None,
    )
    .expect("first");
    let second = ingest_at(
        &vault,
        &input("anchor-b", [1.0, 0.0], [0.0, 1.0]).with_anchor(speaker("bob")),
        EpochSecs(200),
        None,
    )
    .expect("second");
    let first_id = new_id(first);
    let second_id = new_id(second);

    let report = dedup_audit(&vault, second_id).expect("audit conflict");

    assert_eq!(report.merges.len(), 0);
    assert_eq!(report.anchor_conflict_blocks, vec![first_id]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn audit_and_undo_reject_dedup_undo_entry_missing_restored() {
    let root = test_root("dedup-audit-missing-restored");
    let vault = durable_vault(&root);
    let first = ingest_at(
        &vault,
        &input("missing-restored", [1.0, 0.0], [1.0, 0.0]),
        EpochSecs(100),
        None,
    )
    .expect("ingest");
    let id = new_id(first);
    let token = dedup_audit(&vault, id).expect("audit").reversal_token;
    // A foreign/older writer records a DedupUndo entry without the mandatory
    // restored list, through the real commit path so the chain stays intact.
    let payload = serde_json::to_vec(&serde_json::json!({
        "dedup_result": "DedupUndo",
        "reversal": token,
    }))
    .expect("encode payload");
    vault
        .commit_dedup_undo(Vec::new(), Vec::new(), Vec::new(), id, payload)
        .expect("commit undo entry without restored");

    let audit_error = dedup_audit(&vault, id).expect_err("audit fails loud");
    let undo_error = dedup_undo(&vault, &token).expect_err("undo fails loud");

    assert_eq!(audit_error.code, "CALYX_LEDGER_CORRUPT");
    assert_eq!(undo_error.code, "CALYX_LEDGER_CORRUPT");
    assert!(audit_error.message.contains("missing restored"));
    assert!(undo_error.message.contains("missing restored"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn audit_rejects_broken_ledger_chain() {
    let mut rows = ledger_rows_for_test(2);
    rows[1].1[8] ^= 1;

    let error = ensure_ledger_chain(&rows).expect_err("broken chain rejected");

    assert_eq!(error.code, "CALYX_LEDGER_CHAIN_BROKEN");
}

fn durable_vault(root: &std::path::Path) -> AsterVault {
    durable_vault_with_policy(root, tct_policy())
}

fn durable_vault_with_policy(root: &std::path::Path, dedup_policy: DedupPolicy) -> AsterVault {
    AsterVault::open(
        root,
        vault_id(),
        b"dedup-audit-test-salt".to_vec(),
        VaultOptions {
            dedup_policy: Some(dedup_policy),
            ..VaultOptions::default()
        },
    )
    .expect("open vault")
}

fn tct_policy() -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![slot(0)],
            TauStrategy::PerSlot(vec![(slot(0), 0.90)]),
            DedupAction::RecurrenceSeries,
        )
        .expect("policy"),
    )
}

fn input(name: &str, content: [f32; 2], temporal: [f32; 2]) -> IngestInput {
    IngestInput::new(name.as_bytes().to_vec(), 41, Modality::Text)
        .with_slot(
            slot(0),
            SlotVector::Dense {
                dim: 2,
                data: content.to_vec(),
            },
        )
        .with_slot(
            slot(20),
            SlotVector::Dense {
                dim: 2,
                data: temporal.to_vec(),
            },
        )
        .with_temporal_slot(slot(20))
}

fn speaker(name: &str) -> Anchor {
    Anchor {
        kind: AnchorKind::SpeakerMatch,
        value: AnchorValue::Text(name.to_string()),
        source: "synthetic-dedup-audit".to_string(),
        observed_at: 100,
        confidence: 1.0,
    }
}

fn new_id(result: DedupResult) -> CxId {
    match result {
        DedupResult::New(id) => id,
        DedupResult::DedupMerge { .. } | DedupResult::ExactDuplicate(_) => {
            panic!("expected new id")
        }
    }
}

fn merge_occurrence(result: DedupResult) -> u64 {
    match result {
        DedupResult::DedupMerge { occurrence, .. } => occurrence.0,
        DedupResult::New(_) | DedupResult::ExactDuplicate(_) => panic!("expected merge"),
    }
}

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

fn other_vault_id() -> VaultId {
    let mut bytes = vault_id().as_ulid().to_bytes();
    bytes[15] ^= 1;
    VaultId::from_ulid(Ulid::from_bytes(bytes))
}

fn test_root(name: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    std::env::temp_dir().join(format!("{name}-{}-{nonce}", std::process::id()))
}

fn ledger_rows_for_test(count: u8) -> Vec<(u64, Vec<u8>)> {
    let mut appender = LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(10))
        .expect("open appender");
    for seq in 0..count {
        appender
            .append(
                EntryKind::Ingest,
                SubjectId::Cx(CxId::from_bytes([seq; 16])),
                format!("payload-{seq}").into_bytes(),
                ActorId::Service("dedup-audit-test".to_string()),
            )
            .expect("append ledger");
    }
    appender
        .store()
        .scan()
        .expect("scan ledger")
        .into_iter()
        .map(|row| (row.seq, row.bytes))
        .collect()
}

fn restore_snapshot_base_bytes<C>(vault: &AsterVault<C>, target: CxId) -> BTreeMap<CxId, Vec<u8>>
where
    C: Clock,
{
    dedup_ledger_entries(vault)
        .expect("ledger entries")
        .into_iter()
        .filter(|entry| entry.payload.dedup_into_id == Some(target))
        .filter_map(|entry| entry.payload.restore)
        .map(|restore| {
            (
                restore.merged_from,
                encode_constellation_base(&restore.candidate).expect("snapshot base bytes"),
            )
        })
        .collect()
}
