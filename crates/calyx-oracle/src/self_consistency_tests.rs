use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, base_key, ledger_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, VaultId, VaultStore};
use proptest::prelude::*;
use serde_json::json;

use super::*;
use crate::CALYX_ORACLE_NO_RECURRENCE;

const DOMAIN: &str = "issue430";

#[test]
fn flakiness_20_pairs_18_agree_is_point_one() {
    let vault = vault_with_series(&[
        vec![v("pass", None); 6],
        vec![v("pass", None), v("pass", None), v("fail", None)],
        vec![v("edge-a", None); 2],
        vec![v("edge-b", None); 2],
    ]);

    let result = oracle_self_consistency(&vault, DomainId::from(DOMAIN), &FixedClock::new(100))
        .expect("measure self consistency");

    assert_close(result.flakiness, 0.1, 0.001);
    assert_eq!(result.validity, 0.0);
    assert_eq!(result.ceiling, 0.0);
    assert!(result.provisional);
    assert!(result.provenance.is_some());
}

#[test]
fn perfect_ground_truth_tracking_has_unit_validity_and_capped_ceiling() {
    let mut series = vec![
        vec![v("pass", Some("pass")); 6],
        vec![
            v("pass", Some("pass")),
            v("pass", Some("pass")),
            v("fail", Some("fail")),
        ],
        vec![v("pass", Some("pass")); 2],
        vec![v("fail", Some("fail")); 2],
    ];
    for idx in 0..37 {
        let label = if idx % 2 == 0 { "pass" } else { "fail" };
        series.push(vec![v(label, Some(label))]);
    }
    let vault = vault_with_series(&series);

    let result = oracle_self_consistency(&vault, DomainId::from(DOMAIN), &FixedClock::new(200))
        .expect("measure self consistency");

    assert_close(result.flakiness, 0.1, 0.001);
    assert_close(result.validity, 1.0, 0.001);
    assert_close(result.ceiling, 0.9, 0.001);
    assert!(!result.provisional);
}

#[test]
fn ledger_row_is_written_after_successful_measurement() {
    let vault = vault_with_series(&[
        vec![v("pass", None); 6],
        vec![v("pass", None); 3],
        vec![v("pass", None); 2],
    ]);

    let result = oracle_self_consistency(&vault, DomainId::from(DOMAIN), &FixedClock::new(300))
        .expect("measure self consistency");
    let ledger = result.provenance.expect("ledger provenance");
    let bytes = vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Ledger,
            &ledger_key(ledger.seq),
        )
        .expect("read ledger")
        .expect("ledger row");
    let text = String::from_utf8_lossy(&bytes);

    assert!(text.contains("oracle_self_consistency_v1"));
}

#[test]
fn zero_recurrence_pairs_fail_closed() {
    let vault = vault_with_series(&[vec![v("pass", None)]]);
    let error = oracle_self_consistency(&vault, DomainId::from(DOMAIN), &FixedClock::new(400))
        .expect_err("below quorum");

    assert_eq!(error.code(), CALYX_ORACLE_NO_RECURRENCE);
}

#[test]
fn nine_recurrence_pairs_fail_closed() {
    let vault = vault_with_series(&[vec![v("pass", None); 4], vec![v("pass", None); 3]]);
    let error = oracle_self_consistency(&vault, DomainId::from(DOMAIN), &FixedClock::new(500))
        .expect_err("below quorum");

    assert_eq!(error.code(), CALYX_ORACLE_NO_RECURRENCE);
}

#[test]
fn missing_ground_truth_is_provisional_zero_validity() {
    let vault = vault_with_series(&[vec![v("pass", None); 6]]);
    let result = oracle_self_consistency(&vault, DomainId::from(DOMAIN), &FixedClock::new(600))
        .expect("measure self consistency");

    assert_close(result.flakiness, 0.0, 0.001);
    assert_eq!(result.validity, 0.0);
    assert_eq!(result.ceiling, 0.0);
    assert!(result.provisional);
}

#[test]
#[ignore = "manual FSV fixture for issue #430 durable readbacks"]
fn issue430_oracle_self_consistency_fsv_fixture() {
    let root =
        PathBuf::from(std::env::var("CALYX_ISSUE430_FSV_ROOT").expect("CALYX_ISSUE430_FSV_ROOT"));
    fs::create_dir_all(&root).expect("create fsv root");
    let happy = happy_series();
    let cases = [
        ("happy", "issue430-happy", happy.as_slice()),
        (
            "zero-pairs",
            "issue430-zero-pairs",
            &[vec![v("pass", None)]][..],
        ),
        (
            "nine-pairs",
            "issue430-nine-pairs",
            &[vec![v("pass", None); 4], vec![v("pass", None); 3]][..],
        ),
        (
            "no-ground-truth",
            "issue430-no-ground-truth",
            &[vec![v("pass", None); 6]][..],
        ),
    ];
    let mut manifests = Vec::new();
    for (case, domain, series) in cases {
        let vault_dir = root.join(case).join("vault");
        let cx_ids = durable_vault_with_series(&vault_dir, domain, series);
        manifests.push(json!({
            "case": case,
            "domain": domain,
            "vault": vault_dir,
            "vault_id": vault_id().to_string(),
            "salt": "issue430-salt",
            "cx_ids": cx_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
        }));
    }
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "issue": 430,
            "expected_happy": {
                "pair_count": 20,
                "agreement_pairs": 18,
                "flakiness": 0.1,
                "validity": 1.0,
                "ceiling": 0.9,
                "provisional": false
            },
            "expected_edges": {
                "zero-pairs": CALYX_ORACLE_NO_RECURRENCE,
                "nine-pairs": CALYX_ORACLE_NO_RECURRENCE,
                "no-ground-truth": {
                    "flakiness": 0.0,
                    "validity": 0.0,
                    "ceiling": 0.0,
                    "provisional": true
                }
            },
            "cases": manifests,
        }))
        .expect("manifest json"),
    )
    .expect("write manifest");
}

proptest! {
    #[test]
    fn ceiling_stays_bounded(
        flakiness in 0.0f32..=1.0,
        validity in 0.0f32..=1.0,
    ) {
        let result = OracleSelfConsistency::with_provenance(
            flakiness,
            validity,
            false,
            Some(LedgerRef { seq: 7, hash: [7; 32] }),
        );

        prop_assert!(result.ceiling >= 0.0);
        prop_assert!(result.ceiling <= 1.0);
        prop_assert!(result.ceiling <= validity + f32::EPSILON);
        prop_assert!(result.ceiling <= (1.0 - flakiness) + f32::EPSILON);
    }
}

#[derive(Clone)]
struct Row {
    verdict: &'static str,
    truth: Option<&'static str>,
}

fn v(verdict: &'static str, truth: Option<&'static str>) -> Row {
    Row { verdict, truth }
}

fn vault_with_series(series: &[Vec<Row>]) -> AsterVault<FixedClock> {
    let vault = AsterVault::with_clock(vault_id(), b"issue430-salt", FixedClock::new(1));
    for (cx_idx, rows) in series.iter().enumerate() {
        let cx_id = CxId::from_bytes([cx_idx as u8 + 1; 16]);
        let cx = constellation(cx_id);
        vault
            .write_cf(
                ColumnFamily::Base,
                base_key(cx_id),
                encode::encode_constellation_base(&cx).expect("encode base"),
            )
            .expect("write base");
        for (occ_idx, row) in rows.iter().enumerate() {
            let occurrence = Occurrence {
                id: OccurrenceId(occ_idx as u64),
                t_k: EpochSecs(1_000 + occ_idx as i64),
                context: OccurrenceContext::new(context(row)).expect("context"),
            };
            vault
                .write_cf(
                    ColumnFamily::Recurrence,
                    recurrence_key(cx_id, occ_idx as u64),
                    encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence))
                        .expect("encode recurrence"),
                )
                .expect("write recurrence");
        }
    }
    vault
}

fn durable_vault_with_series(vault_dir: &Path, domain: &str, series: &[Vec<Row>]) -> Vec<CxId> {
    assert!(
        !vault_dir.exists(),
        "fresh FSV vault path required: {}",
        vault_dir.display()
    );
    fs::create_dir_all(vault_dir).expect("create vault dir");
    let vault = AsterVault::new_durable(
        vault_dir,
        vault_id(),
        b"issue430-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let mut cx_ids = Vec::new();
    for (cx_idx, rows) in series.iter().enumerate() {
        let cx_id = CxId::from_bytes([cx_idx as u8 + 1; 16]);
        cx_ids.push(cx_id);
        let cx = constellation_for_domain(cx_id, domain);
        vault
            .write_cf(
                ColumnFamily::Base,
                base_key(cx_id),
                encode::encode_constellation_base(&cx).expect("encode base"),
            )
            .expect("write durable base");
        for (occ_idx, row) in rows.iter().enumerate() {
            let occurrence = Occurrence {
                id: OccurrenceId(occ_idx as u64),
                t_k: EpochSecs(1_000 + occ_idx as i64),
                context: OccurrenceContext::new(context(row)).expect("context"),
            };
            vault
                .write_cf(
                    ColumnFamily::Recurrence,
                    recurrence_key(cx_id, occ_idx as u64),
                    encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence))
                        .expect("encode recurrence"),
                )
                .expect("write durable recurrence");
        }
    }
    vault.flush().expect("flush durable vault");
    drop(vault);
    cx_ids
}

fn happy_series() -> Vec<Vec<Row>> {
    let mut series = vec![
        vec![v("pass", Some("pass")); 6],
        vec![
            v("pass", Some("pass")),
            v("pass", Some("pass")),
            v("fail", Some("fail")),
        ],
        vec![v("pass", Some("pass")); 2],
        vec![v("fail", Some("fail")); 2],
    ];
    for idx in 0..37 {
        let label = if idx % 2 == 0 { "pass" } else { "fail" };
        series.push(vec![v(label, Some(label))]);
    }
    series
}

fn context(row: &Row) -> Vec<u8> {
    let mut value = json!({
        "oracle_verdict": { "value": { "text": row.verdict } },
        "outcome_anchor": { "value": { "text": row.verdict } }
    });
    if let Some(truth) = row.truth {
        value["ground_truth_anchor"] = json!({ "value": { "text": truth } });
    }
    serde_json::to_vec(&value).expect("context json")
}

fn constellation(cx_id: CxId) -> calyx_core::Constellation {
    constellation_for_domain(cx_id, DOMAIN)
}

fn constellation_for_domain(cx_id: CxId, domain: &str) -> calyx_core::Constellation {
    let mut metadata = BTreeMap::new();
    metadata.insert(ORACLE_DOMAIN_METADATA_KEY.to_string(), domain.to_string());
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 1,
        input_ref: InputRef {
            hash: [cx_id.as_bytes()[0]; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Structured,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

fn assert_close(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual {actual}, expected {expected}"
    );
}
