use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use calyx_anneal::{
    AnnealLedger, AnnealLedgerAction, BaseFaultEvent, BaseShard, CALYX_ANNEAL_ALERT_WRITE_FAILED,
    CALYX_ANNEAL_RESTORE_FAILED, RestoreCommand, RestoreConfig, RestoreOutcome, ShardId,
    alert_operator, attempt_restore, base_shard_checksum, clear_reads_on_range,
    fail_reads_on_range, install_recorded_read_barriers, record_base_shard_checksum,
    verify_base_shards,
};
use calyx_aster::cf::{ColumnFamily, KeyRange, base_key};
use calyx_aster::mvcc::CALYX_ASTER_BASE_CORRUPT;
use calyx_aster::vault::AsterVault;
use calyx_core::{CxId, FixedClock};
use calyx_ledger::{ActorId, LedgerAppender, MemoryLedgerStore};
use proptest::prelude::*;

mod fsv_support;
use fsv_support::vault_id;

const TEST_TS: u64 = 1_785_600_403;

#[test]
fn corrupt_base_shard_installs_barrier_and_preserves_outside_reads() {
    let clock = FixedClock::new(TEST_TS);
    let vault = source_vault(clock);
    let blocked = cx(0x10);
    let outside = cx(0x20);
    write_base(&vault, blocked, b"base-row-blocked");
    write_base(&vault, outside, b"base-row-outside");
    let range = cx_range(blocked);
    let actual = base_shard_checksum(&vault, &range).unwrap();
    let shard = BaseShard::new(ShardId::new("shard_10"), range, flip(actual));
    record_base_shard_checksum(&vault, &shard, &clock).unwrap();

    let events = verify_base_shards(&vault, &clock).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0],
        BaseFaultEvent::corrupt(shard.clone(), actual, TEST_TS)
    );

    fail_reads_on_range(&vault, &events[0]).unwrap();
    let err = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(blocked))
        .expect_err("blocked base read fails closed");
    assert_eq!(err.code, CALYX_ASTER_BASE_CORRUPT);
    assert_eq!(
        vault
            .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(outside))
            .unwrap(),
        Some(b"base-row-outside".to_vec())
    );
    let checksum_row = checksum_row_text(&vault);
    assert!(checksum_row.contains("\"barrier_installed\":true"));
    assert!(checksum_row.contains(&hex(&actual)));
    assert!(vault.remove_read_barrier("shard_10"));
    assert_eq!(install_recorded_read_barriers(&vault).unwrap(), 1);
    assert_eq!(
        vault
            .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(blocked))
            .expect_err("recorded barrier reinstalls")
            .code,
        CALYX_ASTER_BASE_CORRUPT
    );

    clear_reads_on_range(&vault, &shard, &clock).unwrap();
    assert_eq!(
        vault
            .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(blocked))
            .unwrap(),
        Some(b"base-row-blocked".to_vec())
    );
    assert!(checksum_row_text(&vault).contains("\"barrier_installed\":false"));
}

#[test]
fn alert_operator_writes_ledger_and_jsonl_even_when_later_alert_write_fails() {
    let clock = FixedClock::new(TEST_TS);
    let shard = BaseShard::new(ShardId::new("shard_alert"), cx_range(cx(0x30)), [1; 32]);
    let event = BaseFaultEvent::corrupt(shard, [2; 32], TEST_TS);
    let mut ledger = memory_ledger(clock);
    let dir = temp_dir("alerts");
    let alerts = dir.join("alerts.jsonl");

    alert_operator(&event, &mut ledger, &alerts).unwrap();
    let text = fs::read_to_string(&alerts).unwrap();
    assert!(text.contains("\"action\":\"base_corrupt_alert\""));
    assert!(text.contains("\"shard_id\":\"shard_alert\""));
    assert_eq!(
        ledger.read_recent(1).unwrap()[0].action,
        AnnealLedgerAction::BaseCorruptAlert
    );

    let blocked_path = dir.join("not-a-file");
    fs::create_dir_all(&blocked_path).unwrap();
    let err = alert_operator(&event, &mut ledger, &blocked_path)
        .expect_err("directory alert path fails after ledger write");
    assert_eq!(err.code, CALYX_ANNEAL_ALERT_WRITE_FAILED);
    let recent = ledger.read_recent(2).unwrap();
    assert_eq!(
        recent.last().unwrap().action,
        AnnealLedgerAction::BaseCorruptAlert
    );
}

#[test]
fn restore_edges_operator_required_and_command_failure() {
    let required =
        attempt_restore(ShardId::new("shard_0"), &RestoreConfig::operator_required()).unwrap();
    assert_eq!(
        required,
        RestoreOutcome::OperatorRequired {
            shard_id: ShardId::new("shard_0")
        }
    );
    let failed = attempt_restore(
        ShardId::new("shard_0"),
        &RestoreConfig {
            auto_restore: true,
            command: Some(RestoreCommand {
                program: "calyx-definitely-missing-restore-command".to_string(),
                args: Vec::new(),
            }),
        },
    )
    .expect_err("missing restore command fails closed");
    assert_eq!(failed.code, CALYX_ANNEAL_RESTORE_FAILED);
}

#[test]
fn all_corrupt_shards_fail_reads_closed() {
    let clock = FixedClock::new(TEST_TS);
    let vault = source_vault(clock);
    let ids = [cx(0x40), cx(0x50)];
    for id in ids {
        write_base(&vault, id, &[id.as_bytes()[0]]);
        let range = cx_range(id);
        let shard = BaseShard::new(
            ShardId::new(format!("shard_{:02x}", id.as_bytes()[0])),
            range,
            [0; 32],
        );
        record_base_shard_checksum(&vault, &shard, &clock).unwrap();
    }

    let events = verify_base_shards(&vault, &clock).unwrap();
    assert_eq!(events.len(), 2);
    for event in &events {
        fail_reads_on_range(&vault, event).unwrap();
    }
    for id in ids {
        assert_eq!(
            vault
                .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(id))
                .expect_err("all corrupt base ranges fail closed")
                .code,
            CALYX_ASTER_BASE_CORRUPT
        );
    }
}

proptest! {
    #[test]
    fn barrier_is_reversible(byte in 1u8..=200) {
        let clock = FixedClock::new(TEST_TS);
        let vault = source_vault(clock);
        let id = cx(byte);
        write_base(&vault, id, b"prop-row");
        let range = cx_range(id);
        let actual = base_shard_checksum(&vault, &range).unwrap();
        let shard = BaseShard::new(ShardId::new(format!("shard_{byte}")), range, flip(actual));
        let event = BaseFaultEvent::corrupt(shard.clone(), actual, TEST_TS);

        fail_reads_on_range(&vault, &event).unwrap();
        prop_assert_eq!(
            vault.read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(id))
                .expect_err("blocked").code,
            CALYX_ASTER_BASE_CORRUPT
        );
        clear_reads_on_range(&vault, &shard, &clock).unwrap();
        prop_assert_eq!(
            vault.read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(id)).unwrap(),
            Some(b"prop-row".to_vec())
        );
    }
}

fn source_vault(clock: FixedClock) -> AsterVault<FixedClock> {
    AsterVault::with_clock(vault_id(), b"issue403-restore-salt".to_vec(), clock)
}

fn write_base(vault: &AsterVault<FixedClock>, id: CxId, value: &[u8]) {
    vault
        .write_cf(ColumnFamily::Base, base_key(id), value.to_vec())
        .unwrap();
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

fn cx_range(id: CxId) -> KeyRange {
    let mut end = id.as_bytes().to_vec();
    end[15] = end[15].saturating_add(1);
    KeyRange {
        start: base_key(id),
        end: Some(end),
    }
}

fn flip(mut value: [u8; 32]) -> [u8; 32] {
    value[0] ^= 0xff;
    value
}

fn checksum_row_text(vault: &AsterVault<FixedClock>) -> String {
    let rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealChecksums)
        .unwrap();
    assert_eq!(rows.len(), 1);
    String::from_utf8(rows[0].1.clone()).unwrap()
}

fn memory_ledger(clock: FixedClock) -> AnnealLedger<MemoryLedgerStore, FixedClock> {
    let appender = LedgerAppender::open(MemoryLedgerStore::default(), clock).unwrap();
    AnnealLedger::new(
        appender,
        ActorId::Service("calyx-anneal-restore-test".to_string()),
    )
    .unwrap()
}

fn temp_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "calyx-issue403-restore-{label}-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);
