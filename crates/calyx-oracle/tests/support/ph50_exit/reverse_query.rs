use std::collections::BTreeMap;
use std::path::PathBuf;

use calyx_aster::cf::{ColumnFamily, base_key, ledger_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{
    AnchorValue, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality,
    content_address,
};
use calyx_oracle::{
    DomainId, ORACLE_ACTION_METADATA_KEY, ORACLE_DOMAIN_METADATA_KEY, ORACLE_EFFECT_METADATA_KEY,
    ORACLE_STRUCTURAL_CONFIDENCE_METADATA_KEY, reverse_query,
};
use serde::Serialize;
use serde_json::json;

use super::{REVERSE_DOMAIN, vault_id};

#[derive(Serialize)]
pub(super) struct ReverseReadback {
    domain: &'static str,
    answer: &'static str,
    causes: Vec<calyx_oracle::Cause>,
    ungrounded_answer: &'static str,
    ungrounded_causes: Vec<calyx_oracle::Cause>,
    missing_answer_error_code: String,
    pub(super) planted_ledger_b3: String,
    pub(super) ungrounded_ledger_b3: String,
}

pub(super) fn reverse_cases(vault_dir: PathBuf, clock: &FixedClock) -> ReverseReadback {
    let vault = durable_vault(&vault_dir, b"ph50-reverse");
    write_recurrence_edge(&vault, "code_change_X", "test_failure_Y", 20);
    write_structural_edge(&vault, "structural_A", "structural_B", 0.41);
    vault.flush().expect("flush reverse seed rows");

    let causes = reverse_query(
        &vault,
        &AnchorValue::Text("test_failure_Y".to_string()),
        DomainId::from(REVERSE_DOMAIN),
        clock,
    )
    .expect("planted reverse query");
    assert_eq!(causes[0].action_or_event, "code_change_X");
    assert!(!causes[0].provisional);
    let planted_ledger_b3 = ledger_b3(&vault, &causes[0].provenance);

    let ungrounded = reverse_query(
        &vault,
        &AnchorValue::Text("structural_B".to_string()),
        DomainId::from(REVERSE_DOMAIN),
        clock,
    )
    .expect("ungrounded reverse query");
    assert!(ungrounded.iter().all(|cause| cause.provisional));
    let ungrounded_ledger_b3 = ledger_b3(&vault, &ungrounded[0].provenance);

    let missing = reverse_query(
        &vault,
        &AnchorValue::Text("absent_effect".to_string()),
        DomainId::from(REVERSE_DOMAIN),
        clock,
    )
    .expect_err("missing answer");
    assert_eq!(missing.code(), calyx_oracle::CALYX_ORACLE_DOMAIN_NOT_FOUND);
    vault.flush().expect("flush reverse ledger rows");
    ReverseReadback {
        domain: REVERSE_DOMAIN,
        answer: "test_failure_Y",
        causes,
        ungrounded_answer: "structural_B",
        ungrounded_causes: ungrounded,
        missing_answer_error_code: missing.code().to_string(),
        planted_ledger_b3,
        ungrounded_ledger_b3,
    }
}

fn write_recurrence_edge(vault: &AsterVault, action: &str, outcome: &str, count: u64) {
    let cx_id = cx_from(action, outcome);
    write_base(vault, cx_id, action, None);
    for index in 0..count {
        let context = serde_json::to_vec(&json!({
            "action": action,
            "consequences": [{
                "action_or_event": outcome,
                "domain": REVERSE_DOMAIN,
                "outcome": {"value": {"text": outcome}},
                "grounded": true,
                "provisional": false
            }]
        }))
        .expect("edge context");
        let occurrence = Occurrence {
            id: OccurrenceId(index),
            t_k: EpochSecs(1_000 + index as i64),
            context: OccurrenceContext::new(context).expect("occurrence context"),
        };
        vault
            .write_cf(
                ColumnFamily::Recurrence,
                recurrence_key(cx_id, index),
                encode_recurrence_row(&StoredRecurrenceRow::Occurrence(occurrence))
                    .expect("encode recurrence"),
            )
            .expect("write recurrence");
    }
}

fn write_structural_edge(vault: &AsterVault, action: &str, outcome: &str, confidence: f32) {
    write_base(
        vault,
        cx_from(action, "structural"),
        action,
        Some((AnchorValue::Text(outcome.to_string()), confidence)),
    );
}

fn write_base(
    vault: &AsterVault,
    cx_id: CxId,
    action: &str,
    structural: Option<(AnchorValue, f32)>,
) {
    let mut metadata = BTreeMap::from([
        (
            ORACLE_DOMAIN_METADATA_KEY.to_string(),
            REVERSE_DOMAIN.to_string(),
        ),
        (ORACLE_ACTION_METADATA_KEY.to_string(), action.to_string()),
    ]);
    let mut flags = CxFlags::default();
    if let Some((answer, confidence)) = structural {
        flags.ungrounded = true;
        metadata.insert(
            ORACLE_EFFECT_METADATA_KEY.to_string(),
            serde_json::to_string(&answer).expect("anchor json"),
        );
        metadata.insert(
            ORACLE_STRUCTURAL_CONFIDENCE_METADATA_KEY.to_string(),
            confidence.to_string(),
        );
    }
    let cx = Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 50,
        created_at: 439,
        input_ref: InputRef {
            hash: [cx_id.as_bytes()[0]; 32],
            pointer: Some("synthetic://ph50-exit-reverse".to_string()),
            redacted: true,
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
        flags,
    };
    vault
        .write_cf(
            ColumnFamily::Base,
            base_key(cx_id),
            encode::encode_constellation_base(&cx).expect("encode base"),
        )
        .expect("write base");
}

fn durable_vault(path: &std::path::Path, salt: &[u8]) -> AsterVault {
    AsterVault::new_durable(path, vault_id(), salt.to_vec(), VaultOptions::default())
        .expect("open durable vault")
}

fn ledger_b3(vault: &AsterVault, ledger_ref: &LedgerRef) -> String {
    let row = vault
        .read_cf_at(
            vault.latest_seq(),
            ColumnFamily::Ledger,
            &ledger_key(ledger_ref.seq),
        )
        .expect("read ledger")
        .expect("ledger row");
    blake3::hash(&row).to_hex().to_string()
}

fn cx_from(left: &str, right: &str) -> CxId {
    CxId::from_bytes(content_address([
        REVERSE_DOMAIN.as_bytes(),
        left.as_bytes(),
        right.as_bytes(),
    ]))
}
