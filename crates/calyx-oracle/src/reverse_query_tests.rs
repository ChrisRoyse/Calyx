use std::collections::BTreeMap;

use calyx_aster::cf::{ColumnFamily, base_key, ledger_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{
    AnchorValue, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, VaultId,
    VaultStore, content_address,
};
use proptest::prelude::*;
use serde_json::json;

use super::*;

const DOMAIN: &str = "reverse-query-fixture";

#[test]
fn grounded_recurrence_recovers_planted_cause_with_ledger() {
    let vault = vault();
    write_recurrence_edge(
        &vault,
        "cause_A",
        AnchorValue::Text("effect_B".to_string()),
        true,
        15,
    );

    let causes = reverse_query(
        &vault,
        &AnchorValue::Text("effect_B".to_string()),
        DomainId::from(DOMAIN),
        &FixedClock::new(44),
    )
    .unwrap();

    assert_eq!(causes.len(), 1);
    assert_eq!(causes[0].action_or_event, "cause_A");
    assert!(!causes[0].provisional);
    assert_close(causes[0].confidence, 15.0 / 16.0);
    assert!(ledger_row(&vault, &causes[0].provenance).is_some());
}

#[test]
fn structural_association_without_recurrence_is_provisional() {
    let vault = vault();
    write_structural_edge(
        &vault,
        "cause_A",
        AnchorValue::Text("effect_B".to_string()),
        0.42,
    );

    let causes = reverse_query(
        &vault,
        &AnchorValue::Text("effect_B".to_string()),
        DomainId::from(DOMAIN),
        &FixedClock::new(45),
    )
    .unwrap();

    assert_eq!(causes.len(), 1);
    assert_eq!(causes[0].action_or_event, "cause_A");
    assert!(causes[0].provisional);
    assert_close(causes[0].confidence, 0.42);
}

#[test]
fn grounded_causes_sort_before_provisional_with_stable_tiebreak() {
    let vault = vault();
    write_recurrence_edge(
        &vault,
        "cause_b",
        AnchorValue::Text("effect".to_string()),
        true,
        2,
    );
    write_recurrence_edge(
        &vault,
        "cause_a",
        AnchorValue::Text("effect".to_string()),
        true,
        2,
    );
    write_structural_edge(
        &vault,
        "cause_z",
        AnchorValue::Text("effect".to_string()),
        1.0,
    );

    let causes = reverse_query(
        &vault,
        &AnchorValue::Text("effect".to_string()),
        DomainId::from(DOMAIN),
        &FixedClock::new(46),
    )
    .unwrap();

    assert_eq!(
        causes
            .iter()
            .map(|cause| (cause.action_or_event.as_str(), cause.provisional))
            .collect::<Vec<_>>(),
        vec![("cause_a", false), ("cause_b", false), ("cause_z", true)]
    );
}

#[test]
fn answer_not_found_fails_closed_as_domain_not_found() {
    let vault = vault();
    write_recurrence_edge(
        &vault,
        "cause_A",
        AnchorValue::Text("other".to_string()),
        true,
        1,
    );

    let error = reverse_query(
        &vault,
        &AnchorValue::Text("missing".to_string()),
        DomainId::from(DOMAIN),
        &FixedClock::new(47),
    )
    .unwrap_err();

    assert_eq!(error.code(), crate::CALYX_ORACLE_DOMAIN_NOT_FOUND);
}

#[test]
fn malformed_recurrence_context_fails_closed_as_no_recurrence() {
    let vault = vault();
    write_base(&vault, "cause_A", "malformed");
    let cx_id = cx_id("cause_A", "malformed");
    vault
        .write_cf(
            ColumnFamily::Recurrence,
            recurrence_key(cx_id, 0),
            encode_recurrence_row(&StoredRecurrenceRow::Occurrence(Occurrence {
                id: OccurrenceId(0),
                t_k: EpochSecs(1),
                context: OccurrenceContext::new(b"not-json".to_vec()).unwrap(),
            }))
            .unwrap(),
        )
        .unwrap();

    let error = reverse_query(
        &vault,
        &AnchorValue::Text("effect_B".to_string()),
        DomainId::from(DOMAIN),
        &FixedClock::new(48),
    )
    .unwrap_err();

    assert_eq!(error.code(), crate::CALYX_ORACLE_NO_RECURRENCE);
}

#[test]
fn cycle_back_edge_terminates_without_duplicate_or_self_cause() {
    let vault = vault();
    write_recurrence_edge(
        &vault,
        "cause_A",
        AnchorValue::Text("effect_B".to_string()),
        true,
        1,
    );
    write_recurrence_edge(
        &vault,
        "effect_B",
        AnchorValue::Text("cause_A".to_string()),
        true,
        1,
    );

    let causes = reverse_query(
        &vault,
        &AnchorValue::Text("effect_B".to_string()),
        DomainId::from(DOMAIN),
        &FixedClock::new(49),
    )
    .unwrap();

    assert_eq!(causes.len(), 1);
    assert_eq!(causes[0].action_or_event, "cause_A");
}

proptest! {
    #[test]
    fn grounded_recurrence_is_never_marked_provisional(count in 1_u64..20) {
        let vault = vault();
        write_recurrence_edge(
            &vault,
            "cause_prop",
            AnchorValue::Text("effect_prop".to_string()),
            true,
            count,
        );

        let causes = reverse_query(
            &vault,
            &AnchorValue::Text("effect_prop".to_string()),
            DomainId::from(DOMAIN),
            &FixedClock::new(50),
        ).unwrap();

        prop_assert_eq!(causes.len(), 1);
        prop_assert!(!causes[0].provisional);
    }
}

fn write_recurrence_edge(
    vault: &AsterVault<FixedClock>,
    from: &str,
    outcome: AnchorValue,
    grounded: bool,
    count: u64,
) {
    let series_key = format!("{from}-{}", anchor_slug(&outcome));
    write_base(vault, from, &series_key);
    let cx_id = cx_id(from, &series_key);
    for index in 0..count {
        let occurrence = Occurrence {
            id: OccurrenceId(index),
            t_k: EpochSecs(1_000 + index as i64),
            context: OccurrenceContext::new(edge_context(from, outcome.clone(), grounded)).unwrap(),
        };
        vault
            .write_cf(
                ColumnFamily::Recurrence,
                recurrence_key(cx_id, index),
                encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence)).unwrap(),
            )
            .unwrap();
    }
}

fn write_structural_edge(
    vault: &AsterVault<FixedClock>,
    action: &str,
    answer: AnchorValue,
    confidence: f32,
) {
    let series_key = format!("{action}-structural");
    let id = cx_id(action, &series_key);
    let mut cx = fixture_constellation(vault.vault_id(), id, action);
    cx.flags.ungrounded = true;
    cx.metadata.insert(
        ORACLE_EFFECT_METADATA_KEY.to_string(),
        serde_json::to_string(&answer).unwrap(),
    );
    cx.metadata.insert(
        ORACLE_STRUCTURAL_CONFIDENCE_METADATA_KEY.to_string(),
        confidence.to_string(),
    );
    vault
        .write_cf(
            ColumnFamily::Base,
            base_key(id),
            encode::encode_constellation_base(&cx).unwrap(),
        )
        .unwrap();
}

fn write_base(vault: &AsterVault<FixedClock>, action: &str, series_key: &str) {
    let id = cx_id(action, series_key);
    vault
        .write_cf(
            ColumnFamily::Base,
            base_key(id),
            encode::encode_constellation_base(&fixture_constellation(vault.vault_id(), id, action))
                .unwrap(),
        )
        .unwrap();
}

fn edge_context(from: &str, outcome: AnchorValue, grounded: bool) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "action": from,
        "consequences": [{
            "action_or_event": format!("effect-of-{from}"),
            "domain": DOMAIN,
            "outcome": { "value": outcome },
            "grounded": grounded,
            "provisional": !grounded
        }]
    }))
    .unwrap()
}

fn fixture_constellation(vault_id: VaultId, cx_id: CxId, action: &str) -> Constellation {
    let mut metadata = BTreeMap::new();
    metadata.insert(ORACLE_DOMAIN_METADATA_KEY.to_string(), DOMAIN.to_string());
    metadata.insert(ORACLE_ACTION_METADATA_KEY.to_string(), action.to_string());
    Constellation {
        cx_id,
        vault_id,
        panel_version: 438,
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

fn ledger_row(vault: &AsterVault<FixedClock>, ref_: &LedgerRef) -> Option<Vec<u8>> {
    vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Ledger,
            &ledger_key(ref_.seq),
        )
        .unwrap()
}

fn cx_id(action: &str, series_key: &str) -> CxId {
    CxId::from_bytes(content_address([
        DOMAIN.as_bytes(),
        action.as_bytes(),
        series_key.as_bytes(),
    ]))
}

fn anchor_slug(value: &AnchorValue) -> String {
    serde_json::to_string(value).unwrap()
}

fn vault() -> AsterVault<FixedClock> {
    AsterVault::with_clock(vault_id(), b"reverse-query", FixedClock::new(1))
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn assert_close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1.0e-4, "{actual} != {expected}");
}
