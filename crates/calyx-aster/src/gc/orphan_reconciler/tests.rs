use super::*;
use crate::cf::{ColumnFamily, base_key, slot_key};
use crate::vault::AsterVault;
use crate::vault::encode::{decode_constellation_base, encode_constellation_base};
use calyx_core::{CxFlags, FixedClock, InputRef, LedgerRef, Modality, SlotVector, VaultId};
use calyx_core::{CxId, SlotId};
use calyx_ledger::decode as decode_ledger;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

#[derive(Default)]
struct FakeTarget {
    base: Vec<OrphanBaseEntry>,
    slots: Vec<OrphanIndexEntry>,
    purged: RefCell<BTreeMap<CxId, BTreeSet<SlotId>>>,
    degraded: RefCell<Vec<CxId>>,
    ledger_rows: RefCell<Vec<CxId>>,
}

impl OrphanGcTarget for FakeTarget {
    fn base_entries(&self) -> Result<Vec<OrphanBaseEntry>> {
        Ok(self.base.clone())
    }

    fn slot_index_entries(&self) -> Result<Vec<OrphanIndexEntry>> {
        Ok(self.slots.clone())
    }

    fn purge_orphan_index(&self, cx_id: CxId, slots: &[SlotId]) -> Result<usize> {
        let count = slots.len().max(1);
        self.purged
            .borrow_mut()
            .entry(cx_id)
            .or_default()
            .extend(slots.iter().copied());
        self.ledger_rows.borrow_mut().push(cx_id);
        Ok(count)
    }

    fn flag_orphan_base(&self, cx_id: CxId) -> Result<()> {
        self.degraded.borrow_mut().push(cx_id);
        Ok(())
    }
}

#[test]
fn scan_finds_base_rows_missing_all_expected_slot_entries() {
    let target = FakeTarget {
        base: (1..=5)
            .map(|seed| base(seed, &[0]))
            .collect::<Vec<OrphanBaseEntry>>(),
        slots: vec![slot_entry(1, 0), slot_entry(2, 0), slot_entry(3, 0)],
        ..FakeTarget::default()
    };
    let report = OrphanReconciler::default().scan(&target).unwrap();

    assert_eq!(report.orphan_base, vec![cx(4), cx(5)]);
    assert!(report.orphan_index.is_empty());
    assert_eq!(report.inconsistencies, 2);
    println!(
        "FSV_ISSUE485_ORPHAN_BASE={:?}",
        report
            .orphan_base
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
}

#[test]
fn scan_and_repair_purges_orphan_index_entries_and_writes_ledger_rows() {
    let target = FakeTarget {
        base: vec![base(1, &[0])],
        slots: vec![
            slot_entry(1, 0),
            slot_entry(6, 0),
            slot_entry(7, 0),
            slot_entry(8, 0),
        ],
        ..FakeTarget::default()
    };
    let reconciler = OrphanReconciler::new(Duration::ZERO, 10);
    let report = reconciler.scan(&target).unwrap();
    let repair = reconciler.repair(&target, &report).unwrap();

    assert_eq!(report.orphan_index, vec![cx(6), cx(7), cx(8)]);
    assert_eq!(repair.orphan_index_repaired, 3);
    assert_eq!(repair.repairs_total, 3);
    assert_eq!(target.ledger_rows.borrow().len(), 3);
    assert!(target.purged.borrow().contains_key(&cx(6)));
}

#[test]
fn repair_rate_limit_leaves_remaining_orphans_for_next_run() {
    let target = FakeTarget {
        slots: (1..=10).map(|seed| slot_entry(seed, 0)).collect(),
        ..FakeTarget::default()
    };
    let reconciler = OrphanReconciler::new(Duration::ZERO, 3);
    let report = reconciler.scan(&target).unwrap();
    let repair = reconciler.repair(&target, &report).unwrap();

    assert_eq!(report.inconsistencies, 10);
    assert_eq!(repair.orphan_index_repaired, 3);
    assert_eq!(repair.remaining_inconsistencies, 7);
    assert!(repair.rate_limited);
}

#[test]
fn metrics_text_uses_required_names() {
    let report = OrphanReport {
        orphan_index: vec![cx(1)],
        orphan_base: vec![cx(2)],
        orphan_index_entries: vec![slot_entry(1, 0), slot_entry(1, 1)],
        inconsistencies: 2,
    };
    let metrics = report.to_metrics_text("issue485");

    assert!(metrics.contains("calyx_orphan_index_entries_total{vault=\"issue485\"} 2"));
    assert!(metrics.contains("calyx_orphan_base_entries_total{vault=\"issue485\"} 1"));
}

#[test]
fn vault_target_repairs_real_cf_rows_and_queues_base_rebuild_once() {
    let vault = AsterVault::with_clock(vault_id(), b"issue485-orphan", FixedClock::new(485));
    let slot = SlotId::new(0);
    for seed in 1..=5 {
        vault
            .write_cf(
                ColumnFamily::Base,
                base_key(cx(seed)),
                encode_constellation_base(&constellation(seed, &[slot])).unwrap(),
            )
            .unwrap();
    }
    for seed in [1, 2, 3, 6, 7, 8] {
        vault
            .write_cf(ColumnFamily::slot(slot), slot_key(cx(seed)), vec![seed])
            .unwrap();
    }

    let target = VaultOrphanGcTarget::new(&vault, [slot]);
    let reconciler = OrphanReconciler::new(Duration::ZERO, 10);
    let before_slot_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::slot(slot))
        .unwrap()
        .len();
    let before_ledger_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap()
        .len();
    let report = reconciler.scan(&target).unwrap();
    let repair = reconciler.repair(&target, &report).unwrap();

    assert_eq!(before_slot_rows, 6);
    assert_eq!(report.orphan_index, vec![cx(6), cx(7), cx(8)]);
    assert_eq!(report.orphan_base, vec![cx(4), cx(5)]);
    assert_eq!(repair.repairs_total, 5);
    let after_slot_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::slot(slot))
        .unwrap();
    assert_eq!(after_slot_rows.len(), 3);
    assert!(
        !after_slot_rows
            .iter()
            .any(|(key, _)| key == &slot_key(cx(6)))
    );

    let base4 = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(cx(4)))
        .unwrap()
        .and_then(|bytes| decode_constellation_base(&bytes).ok())
        .unwrap();
    assert!(base4.flags.degraded);
    assert_eq!(
        base4
            .metadata
            .get("gc.orphan_reconciler")
            .map(String::as_str),
        Some("slot_rebuild_queued")
    );
    let replay_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealReplay)
        .unwrap();
    assert_eq!(replay_rows.len(), 2);

    let ledger_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap();
    assert_eq!(ledger_rows.len(), before_ledger_rows + 5);
    let payload_events = ledger_rows
        .iter()
        .filter_map(|(_, bytes)| decode_ledger(bytes).ok())
        .filter_map(|entry| serde_json::from_slice::<serde_json::Value>(&entry.payload).ok())
        .filter_map(|payload| payload.get("event")?.as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    assert!(payload_events.contains("orphan_index_purged"));
    assert!(payload_events.contains("orphan_base_degraded"));

    let repeat_report = reconciler.scan(&target).unwrap();
    assert!(repeat_report.orphan_index.is_empty());
    assert!(repeat_report.orphan_base.is_empty());
    println!(
        "FSV_ISSUE485_REAL_ORPHAN_CF before_slots={before_slot_rows} after_slots={} replay_rows={} ledger_delta={}",
        after_slot_rows.len(),
        replay_rows.len(),
        ledger_rows.len() - before_ledger_rows
    );
}

fn base(seed: u8, slots: &[u16]) -> OrphanBaseEntry {
    OrphanBaseEntry {
        cx_id: cx(seed),
        expected_slots: slots.iter().copied().map(SlotId::new).collect(),
        repair_queued: false,
    }
}

fn slot_entry(seed: u8, slot: u16) -> OrphanIndexEntry {
    OrphanIndexEntry {
        cx_id: cx(seed),
        slot: SlotId::new(slot),
    }
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn constellation(seed: u8, slots: &[SlotId]) -> calyx_core::Constellation {
    let slot_rows = slots
        .iter()
        .copied()
        .map(|slot| {
            (
                slot,
                SlotVector::Dense {
                    dim: 1,
                    data: vec![f32::from(seed)],
                },
            )
        })
        .collect();
    calyx_core::Constellation {
        cx_id: cx(seed),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("synthetic://issue485/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: slot_rows,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}
