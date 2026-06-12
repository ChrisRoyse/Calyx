use std::collections::BTreeMap;

use calyx_aster::cf::{ColumnFamily, base_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{
    AnchorValue, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, VaultId,
    content_address,
};
use proptest::prelude::*;
use serde_json::json;

use super::*;

const DOMAIN: &str = "butterfly-fixture";

#[test]
fn linear_chain_expands_depth_first_with_attenuation() {
    let vault = vault();
    write_edge(&vault, "A", "B", AnchorValue::Text("b".to_string()), true);
    write_edge(&vault, "B", "C", AnchorValue::Text("c".to_string()), true);
    write_edge(&vault, "C", "D", AnchorValue::Text("d".to_string()), true);

    let flat = expand(&vault, &root("A", 1.0, 0), &FixedClock::new(10)).unwrap();

    assert_eq!(
        flat.iter()
            .map(|consequence| consequence.action_or_event.as_str())
            .collect::<Vec<_>>(),
        vec!["B", "C", "D"]
    );
    assert_close(flat[2].confidence, 1.0 * HOP_ATTENUATION.powi(3));
    assert_eq!(flat[2].hop, 3);
    assert!(!is_provisional_ledger_ref(&flat[2].provenance));
}

#[test]
fn select_returns_terminal_branch_matching_target_outcome() {
    let tree = ConsequenceTree {
        root: root("A", 1.0, 0),
        children: vec![
            ConsequenceTree {
                root: consequence("B", "wrong", 0.7, 1),
                children: Vec::new(),
                max_depth: MAX_DEPTH,
            },
            ConsequenceTree {
                root: consequence("C", "target", 0.7, 1),
                children: Vec::new(),
                max_depth: MAX_DEPTH,
            },
        ],
        max_depth: MAX_DEPTH,
    };

    let selected = select(&tree, &AnchorValue::Text("target".to_string())).unwrap();

    assert_eq!(selected.root.action_or_event, "C");
}

#[test]
fn cycle_a_b_a_terminates_without_revisiting_root() {
    let vault = vault();
    write_edge(&vault, "A", "B", AnchorValue::Text("b".to_string()), true);
    write_edge(&vault, "B", "A", AnchorValue::Text("a".to_string()), true);

    let tree = build_tree(&vault, root("A", 1.0, 0), &FixedClock::new(11)).unwrap();

    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].root.action_or_event, "B");
    assert!(tree.children[0].children.is_empty());
}

#[test]
fn edge_pruning_returns_empty_for_no_outgoing_max_hop_and_low_confidence() {
    let vault = vault();

    assert!(
        expand(&vault, &root("none", 1.0, 0), &FixedClock::new(12))
            .unwrap()
            .is_empty()
    );
    assert!(
        expand(&vault, &root("max", 1.0, MAX_DEPTH), &FixedClock::new(12))
            .unwrap()
            .is_empty()
    );
    assert!(
        expand(&vault, &root("low", 0.01, 0), &FixedClock::new(12))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn ungrounded_edge_is_provisional_and_not_traversed() {
    let vault = vault();
    write_edge(
        &vault,
        "A",
        "B",
        AnchorValue::Text("provisional".to_string()),
        false,
    );
    write_edge(&vault, "B", "C", AnchorValue::Text("c".to_string()), true);

    let tree = build_tree(&vault, root("A", 1.0, 0), &FixedClock::new(13)).unwrap();

    assert_eq!(tree.children.len(), 1);
    assert!(is_provisional_ledger_ref(&tree.children[0].root.provenance));
    assert!(tree.children[0].children.is_empty());
}

#[test]
fn malformed_recurrence_row_fails_closed_as_no_recurrence() {
    let vault = vault();
    write_base(&vault, DOMAIN, "A", "bad-row");
    let cx_id = cx_id(DOMAIN, "A", "bad-row");
    vault
        .write_cf(
            ColumnFamily::Recurrence,
            recurrence_key(cx_id, 0),
            b"not-json".to_vec(),
        )
        .unwrap();

    let error = expand(&vault, &root("A", 1.0, 0), &FixedClock::new(14)).unwrap_err();

    assert_eq!(error.code(), crate::CALYX_ORACLE_NO_RECURRENCE);
}

proptest! {
    #[test]
    fn generated_chains_stay_bounded_and_monotone(len in 0_usize..8) {
        let vault = vault();
        for index in 0..len {
            write_edge(
                &vault,
                &format!("N{index}"),
                &format!("N{}", index + 1),
                AnchorValue::Text(format!("outcome-{index}")),
                true,
            );
        }

        let tree = build_tree(&vault, root("N0", 1.0, 0), &FixedClock::new(15)).unwrap();

        assert_tree_bounds(&tree);
    }
}

fn assert_tree_bounds(tree: &ConsequenceTree) {
    for child in &tree.children {
        assert!(child.root.hop <= MAX_DEPTH);
        assert!(child.root.confidence <= tree.root.confidence);
        assert_tree_bounds(child);
    }
}

fn write_edge(
    vault: &AsterVault<FixedClock>,
    from: &str,
    to: &str,
    outcome: AnchorValue,
    grounded: bool,
) {
    let series_key = format!("{from}-{to}");
    write_base(vault, DOMAIN, from, &series_key);
    let cx_id = cx_id(DOMAIN, from, &series_key);
    let occurrence = Occurrence {
        id: OccurrenceId(0),
        t_k: EpochSecs(1_000),
        context: OccurrenceContext::new(edge_context(from, to, outcome, grounded)).unwrap(),
    };
    vault
        .write_cf(
            ColumnFamily::Recurrence,
            recurrence_key(cx_id, 0),
            encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence)).unwrap(),
        )
        .unwrap();
}

fn write_base(vault: &AsterVault<FixedClock>, domain: &str, action: &str, series_key: &str) {
    let id = cx_id(domain, action, series_key);
    vault
        .write_cf(
            ColumnFamily::Base,
            base_key(id),
            encode::encode_constellation_base(&fixture_constellation(
                vault.vault_id(),
                id,
                domain,
                action,
            ))
            .unwrap(),
        )
        .unwrap();
}

fn edge_context(from: &str, to: &str, outcome: AnchorValue, grounded: bool) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "action": from,
        "consequences": [{
            "action_or_event": to,
            "domain": DOMAIN,
            "outcome": { "value": outcome },
            "grounded": grounded
        }]
    }))
    .unwrap()
}

fn fixture_constellation(
    vault_id: VaultId,
    cx_id: CxId,
    domain: &str,
    action: &str,
) -> Constellation {
    let mut metadata = BTreeMap::new();
    metadata.insert(ORACLE_DOMAIN_METADATA_KEY.to_string(), domain.to_string());
    metadata.insert(ORACLE_ACTION_METADATA_KEY.to_string(), action.to_string());
    Constellation {
        cx_id,
        vault_id,
        panel_version: 433,
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
        provenance: ledger(0),
        flags: CxFlags::default(),
    }
}

fn root(action: &str, confidence: f32, hop: u8) -> Consequence {
    Consequence {
        action_or_event: action.to_string(),
        domain: DomainId::from(DOMAIN),
        outcome: AnchorValue::Text(format!("root-{action}")),
        confidence,
        hop,
        provenance: ledger(1),
    }
}

fn consequence(action: &str, outcome: &str, confidence: f32, hop: u8) -> Consequence {
    Consequence {
        action_or_event: action.to_string(),
        domain: DomainId::from(DOMAIN),
        outcome: AnchorValue::Text(outcome.to_string()),
        confidence,
        hop,
        provenance: ledger(2),
    }
}

fn cx_id(domain: &str, action: &str, series_key: &str) -> CxId {
    CxId::from_bytes(content_address([
        domain.as_bytes(),
        action.as_bytes(),
        series_key.as_bytes(),
    ]))
}

fn vault() -> AsterVault<FixedClock> {
    AsterVault::with_clock(vault_id(), b"butterfly", FixedClock::new(1))
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn ledger(seed: u64) -> LedgerRef {
    LedgerRef {
        seq: seed,
        hash: [seed as u8; 32],
    }
}

fn assert_close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1.0e-4, "{actual} != {expected}");
}
