use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{
    Domain, OutcomeAgreement, measure_outcome_agreement, oracle_self_consistency,
    outcome_occurrence_context,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{FREQUENCY_SCALAR, RetentionPolicy, append_occurrence};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, AnchorValue, CxId, VaultStore};
use serde_json::{Value, json};

#[path = "recurrence_anchor_support/mod.rs"]
mod recurrence_anchor_support;
use recurrence_anchor_support::{append_outcomes, base_cx, cx_id, vault_id};

#[test]
#[ignore = "FSV trigger writes durable manual evidence under CALYX_ASSAY_ISSUE387_FSV_DIR"]
fn issue387_oracle_self_consistency_fsv_writes_assay_report() {
    let root = PathBuf::from(
        env::var("CALYX_ASSAY_ISSUE387_FSV_DIR").expect("set CALYX_ASSAY_ISSUE387_FSV_DIR"),
    );
    fs::create_dir_all(&root).expect("create fsv root");
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue387-assay-recurrence-fsv".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");

    let mixed_ids = (1_u8..=5).map(cx_id).collect::<Vec<_>>();
    for cx_id in &mixed_ids {
        vault.put(base_cx(*cx_id)).expect("put mixed base");
    }
    let before = raw_state(&vault);

    for cx_id in &mixed_ids[..3] {
        append_outcomes(&vault, *cx_id, &["agree", "agree", "agree"]);
    }
    for cx_id in &mixed_ids[3..] {
        append_outcomes(&vault, *cx_id, &["agree", "agree", "differ"]);
    }
    vault.flush().expect("flush mixed vault");

    let mixed = Domain::new("issue387-mixed", mixed_ids.clone());
    let score = oracle_self_consistency(&mixed, &vault).expect("mixed score");
    assert!(score > 0.6 && score < 0.9, "mixed score was {score}");

    let wrong_id = cx_id(20);
    vault.put(base_cx(wrong_id)).expect("put wrong base");
    append_wrong_anchor(&vault, wrong_id);
    let wrong_error = measure_outcome_agreement(wrong_id, &vault).expect_err("wrong anchor error");

    let insufficient_id = cx_id(21);
    vault
        .put(base_cx(insufficient_id))
        .expect("put insufficient base");
    append_outcomes(&vault, insufficient_id, &["agree", "agree"]);
    let insufficient =
        measure_outcome_agreement(insufficient_id, &vault).expect("insufficient agreement");

    let no_recurring_id = cx_id(30);
    vault
        .put(base_cx(no_recurring_id))
        .expect("put no-recurring base");
    let no_recurring_score = oracle_self_consistency(
        &Domain::new("issue387-no-recurring", vec![no_recurring_id]),
        &vault,
    )
    .expect("no recurring score");

    let after = raw_state(&vault);
    let report = json!({
        "schema_version": 1,
        "surface": "assay-report",
        "artifact_kind": "ph42.assay-report.v1",
        "source_of_truth": "PH42 persisted artifact",
        "issue": 387,
        "domain": mixed.id,
        "oracle_self_consistency": score,
        "expected_range": { "low_exclusive": 0.6, "high_exclusive": 0.9 },
        "cx_agreements": mixed_ids.iter().map(|cx_id| cx_report(&vault, *cx_id)).collect::<Vec<_>>(),
        "edges": {
            "wrong_outcome_anchor_error": {
                "code": wrong_error.code,
                "message": wrong_error.message,
                "remediation": wrong_error.remediation
            },
            "insufficient_occurrences": insufficient_json(&insufficient),
            "no_recurring_domain_score": no_recurring_score
        },
        "source_of_truth_bytes": {
            "vault_dir": vault_dir.display().to_string(),
            "before": before,
            "after": after
        }
    });

    let artifact = root.join("assay-report.json");
    write_json(&artifact, &report);
    write_blake3_manifest(&root, std::slice::from_ref(&artifact));
    println!("issue387_fsv_root={}", root.display());
    println!("issue387_assay_report={}", artifact.display());
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}

fn cx_report(vault: &AsterVault, cx_id: CxId) -> Value {
    let agreement = measure_outcome_agreement(cx_id, vault).expect("agreement");
    let base = vault.get(cx_id, vault.snapshot()).expect("base");
    json!({
        "cx_id": cx_id.to_string(),
        "frequency_scalar": base.scalars.get(FREQUENCY_SCALAR),
        "agreement": agreement_json(&agreement)
    })
}

fn agreement_json(agreement: &OutcomeAgreement) -> Value {
    match agreement {
        OutcomeAgreement::Consistent { agreement_rate } => {
            json!({ "classification": "consistent", "agreement_rate": agreement_rate })
        }
        OutcomeAgreement::Flaky { agreement_rate } => {
            json!({ "classification": "flaky", "agreement_rate": agreement_rate })
        }
        OutcomeAgreement::Insufficient { n } => json!({ "classification": "insufficient", "n": n }),
    }
}

fn insufficient_json(agreement: &OutcomeAgreement) -> Value {
    assert_eq!(agreement, &OutcomeAgreement::Insufficient { n: 2 });
    agreement_json(agreement)
}

fn append_wrong_anchor(vault: &AsterVault, cx_id: CxId) {
    let context = outcome_occurrence_context(AnchorKind::Reward, AnchorValue::Text("agree".into()))
        .expect("context");
    for index in 0..3 {
        append_occurrence(
            vault,
            cx_id,
            EpochSecs(2_000 + index),
            context.clone(),
            EpochSecs(2_000 + index),
            RetentionPolicy::default(),
        )
        .expect("append wrong anchor");
    }
}

fn raw_state(vault: &AsterVault) -> Value {
    json!({
        "snapshot": vault.snapshot(),
        "base": raw_rows(vault, ColumnFamily::Base),
        "recurrence": raw_rows(vault, ColumnFamily::Recurrence),
        "ledger": raw_rows(vault, ColumnFamily::Ledger)
    })
}

fn raw_rows(vault: &AsterVault, cf: ColumnFamily) -> Value {
    let rows = vault.scan_cf_at(vault.snapshot(), cf).expect("scan cf");
    json!({
        "row_count": rows.len(),
        "rows": rows.iter().map(|(key, value)| {
            json!({ "key_hex": hex(key), "value_hex": hex(value) })
        }).collect::<Vec<_>>()
    })
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

fn write_blake3_manifest(root: &Path, paths: &[PathBuf]) {
    let mut manifest = String::new();
    for path in paths {
        let bytes = fs::read(path).expect("read artifact");
        let name = path.file_name().unwrap().to_string_lossy();
        manifest.push_str(&format!("{}  {name}\n", blake3::hash(&bytes).to_hex()));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), manifest).expect("write blake3 manifest");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
