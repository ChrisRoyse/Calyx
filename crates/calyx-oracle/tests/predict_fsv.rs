use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use calyx_assay::{AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, TrustTag};
use calyx_aster::cf::{ColumnFamily, base_key, recurrence_key};
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::{
    Occurrence, OccurrenceContext, StoredRecurrenceRow, encode_recurrence_row,
};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{
    AnchorKind, AnchorValue, Asymmetry, Clock, CxFlags, CxId, InputRef, LedgerRef, LensId,
    Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, SystemClock,
    VaultId, content_address,
};
use calyx_oracle::{
    Action, CALYX_ORACLE_INSUFFICIENT, CALYX_ORACLE_NO_RECURRENCE, DomainId,
    ORACLE_ACTION_METADATA_KEY, ORACLE_DOMAIN_METADATA_KEY, OracleError, oracle_predict,
    oracle_self_consistency,
};
use serde_json::json;

const DOMAIN: &str = "swe_bench_lite_ph49_fsv";
const FORM_VERSION: u32 = 4_341;
const EXEC_VERSION: u32 = 4_342;
const CEILING: f32 = 0.73;

#[test]
fn ph49_fsv_swe_bench_deficit_refuses_and_caps_predictions() {
    let root = prepare_fsv_root();
    let vault = durable_vault(&root.join("vault"));
    let clock = calyx_testkit::fixed_clock();
    let form_panel = panel(FORM_VERSION, &[1, 2, 3]);
    let exec_panel = panel(EXEC_VERSION, &[11, 12, 13, 14]);

    put_sufficiency(&vault, DOMAIN, &form_panel, 0.46, 1.0, &[0.10, 0.15, 0.21]);
    put_sufficiency(
        &vault,
        DOMAIN,
        &exec_panel,
        1.0,
        1.0,
        &[0.22, 0.25, 0.26, 0.27],
    );
    seed_exact_ceiling(&vault, DOMAIN);

    let insufficient = oracle_predict(
        &vault,
        &action("form_only_patch", form_panel),
        DomainId::from(DOMAIN),
        &clock,
    )
    .expect_err("form-only panel must refuse");
    assert_eq!(insufficient.code(), CALYX_ORACLE_INSUFFICIENT);
    let bound = match insufficient {
        OracleError::Insufficient { bound } => bound,
        other => panic!("expected insufficient, got {}", other.code()),
    };
    assert!((0.40..=0.55).contains(&bound.i_panel_oracle));
    assert!(!bound.per_sensor_deficit.is_empty());
    let deficit_sum: f32 = bound.per_sensor_deficit.iter().map(|(_, gap)| gap).sum();
    assert!((deficit_sum - (1.0 - bound.i_panel_oracle)).abs() <= 0.10);
    print_json(json!({
        "fsv": "ph49_swe_bench_form_only_deficit",
        "error_code": CALYX_ORACLE_INSUFFICIENT,
        "bound": bound,
        "per_sensor_deficit_sum": deficit_sum,
        "expected_deficit_bits": 1.0 - bound.i_panel_oracle,
    }));

    let self_consistency =
        oracle_self_consistency(&vault, DomainId::from(DOMAIN), &clock).expect("ceiling");
    assert!((self_consistency.ceiling - CEILING).abs() < 1.0e-6);
    print_json(json!({
        "fsv": "ph49_self_consistency_ceiling",
        "self_consistency": self_consistency,
        "hand_expected": {
            "pair_count": 1000,
            "agreement_pairs": 730,
            "validity": 1.0,
            "ceiling": CEILING
        }
    }));

    write_ceiling_scan(&vault, &exec_panel, &clock);
    verify_recurrence_14_of_15(&vault, &exec_panel, &clock);
    verify_no_recurrence_edge(&vault, &exec_panel, &clock);
    verify_corrupt_recurrence_edge(&root, &exec_panel, &clock);

    vault.flush().expect("flush fsv vault");
    write_summary(&root);
}

fn write_ceiling_scan(vault: &AsterVault<SystemClock>, panel: &Panel, clock: &dyn Clock) {
    let mut log = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(ceiling_log_path())
        .expect("open ceiling jsonl");
    let planted = [20, 25, 30, 35, 40]
        .into_iter()
        .enumerate()
        .map(|(idx, rows)| {
            let action_id = format!("ceiling_action_{idx:02}");
            seed_prediction_rows(vault, DOMAIN, &action_id, rows, 0);
            (action_id, rows)
        })
        .collect::<Vec<_>>();

    for idx in 0..50 {
        let (action_id, recurrence_rows) = &planted[idx % planted.len()];
        let prediction = oracle_predict(
            vault,
            &action(action_id, panel.clone()),
            DomainId::from(DOMAIN),
            clock,
        )
        .expect("prediction under ceiling");
        assert!(prediction.confidence <= CEILING + f32::EPSILON);
        assert!(prediction.confidence <= prediction.bound.dpi_ceiling + f32::EPSILON);
        let row = json!({
            "idx": idx,
            "action_id": action_id,
            "prediction_outcome": prediction.outcome,
            "confidence": prediction.confidence,
            "ceiling": CEILING,
            "within_ceiling": prediction.confidence <= CEILING + f32::EPSILON,
            "dpi_ceiling": prediction.bound.dpi_ceiling,
            "recurrence_rows": recurrence_rows
        });
        writeln!(log, "{}", serde_json::to_string(&row).unwrap()).expect("write ceiling row");
    }
    print_json(json!({
        "fsv": "ph49_ceiling_scan",
        "path": ceiling_log_path(),
        "rows": 50,
        "expected_all_within_ceiling": true
    }));
}

fn verify_recurrence_14_of_15(vault: &AsterVault<SystemClock>, panel: &Panel, clock: &dyn Clock) {
    let action_id = "recurrence_14_pass_of_15";
    seed_prediction_rows(vault, DOMAIN, action_id, 14, 1);
    let prediction = oracle_predict(
        vault,
        &action(action_id, panel.clone()),
        DomainId::from(DOMAIN),
        clock,
    )
    .expect("14/15 recurrence predicts");
    assert_eq!(prediction.outcome, AnchorValue::Text("Pass".to_string()));
    assert!(prediction.confidence > 0.5);
    assert!(prediction.confidence <= CEILING + f32::EPSILON);
    print_json(json!({
        "fsv": "ph49_recurrence_14_of_15",
        "outcome": prediction.outcome,
        "confidence": prediction.confidence,
        "ceiling": CEILING,
        "source_ledger_seq": prediction.provenance.seq
    }));
}

fn verify_no_recurrence_edge(vault: &AsterVault<SystemClock>, panel: &Panel, clock: &dyn Clock) {
    let error = oracle_predict(
        vault,
        &action("missing_recurrence_action", panel.clone()),
        DomainId::from(DOMAIN),
        clock,
    )
    .expect_err("missing action recurrence");
    assert_eq!(error.code(), CALYX_ORACLE_NO_RECURRENCE);
    print_json(json!({
        "fsv": "ph49_no_recurrence_edge",
        "error_code": error.code(),
        "expected": CALYX_ORACLE_NO_RECURRENCE
    }));
}

fn verify_corrupt_recurrence_edge(root: &Path, panel: &Panel, clock: &dyn Clock) {
    let vault = durable_vault(&root.join("corrupt-vault"));
    put_sufficiency(&vault, DOMAIN, panel, 1.0, 1.0, &[0.25, 0.25, 0.25, 0.25]);
    seed_exact_ceiling(&vault, DOMAIN);
    let cx_id = write_base(&vault, DOMAIN, "corrupt_recurrence", "corrupt-series");
    vault
        .write_cf(
            ColumnFamily::Recurrence,
            recurrence_key(cx_id, 0),
            b"not-a-stored-recurrence-row".to_vec(),
        )
        .expect("write corrupt recurrence");
    let error = oracle_predict(
        &vault,
        &action("corrupt_recurrence", panel.clone()),
        DomainId::from(DOMAIN),
        clock,
    )
    .expect_err("corrupt recurrence must fail closed");
    assert_eq!(error.code(), CALYX_ORACLE_NO_RECURRENCE);
    vault.flush().expect("flush corrupt vault");
    print_json(json!({
        "fsv": "ph49_corrupt_recurrence_edge",
        "error_code": error.code(),
        "expected": CALYX_ORACLE_NO_RECURRENCE
    }));
}

fn seed_exact_ceiling(vault: &AsterVault<SystemClock>, domain: &str) {
    let first = vec![
        Row::truth("Pass"),
        Row::truth("Fail"),
        Row::truth("Fail"),
        Row::truth("Fail"),
        Row::truth("Fail"),
    ];
    write_series(vault, domain, "calibration", "pairs_10_agree_6", &first);

    let mut second = Vec::new();
    second.extend((0..7).map(|_| Row::truth("Pass")));
    second.extend((0..38).map(|_| Row::truth("Fail")));
    write_series(vault, domain, "calibration", "pairs_990_agree_724", &second);
}

fn seed_prediction_rows(
    vault: &AsterVault<SystemClock>,
    domain: &str,
    action_id: &str,
    pass_count: usize,
    fail_count: usize,
) {
    let mut idx = 0;
    for _ in 0..pass_count {
        let key = format!("{action_id}-pass-{idx}");
        write_series(vault, domain, action_id, &key, &[Row::prediction("Pass")]);
        idx += 1;
    }
    for _ in 0..fail_count {
        let key = format!("{action_id}-fail-{idx}");
        write_series(vault, domain, action_id, &key, &[Row::prediction("Fail")]);
        idx += 1;
    }
}

#[derive(Clone)]
struct Row {
    outcome: &'static str,
    truth: Option<&'static str>,
}

impl Row {
    fn truth(outcome: &'static str) -> Self {
        Self {
            outcome,
            truth: Some(outcome),
        }
    }

    fn prediction(outcome: &'static str) -> Self {
        Self {
            outcome,
            truth: None,
        }
    }
}

fn write_series(
    vault: &AsterVault<SystemClock>,
    domain: &str,
    action_id: &str,
    series_key: &str,
    rows: &[Row],
) {
    let cx_id = write_base(vault, domain, action_id, series_key);
    for (occ_idx, row) in rows.iter().enumerate() {
        let occurrence = Occurrence {
            id: OccurrenceId(occ_idx as u64),
            t_k: EpochSecs(1_000 + occ_idx as i64),
            context: OccurrenceContext::new(context(action_id, row)).expect("context"),
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

fn write_base(
    vault: &AsterVault<SystemClock>,
    domain: &str,
    action_id: &str,
    series_key: &str,
) -> CxId {
    let cx_id = CxId::from_bytes(content_address([
        domain.as_bytes(),
        action_id.as_bytes(),
        series_key.as_bytes(),
    ]));
    vault
        .write_cf(
            ColumnFamily::Base,
            base_key(cx_id),
            encode::encode_constellation_base(&constellation(
                vault.vault_id(),
                cx_id,
                domain,
                action_id,
            ))
            .expect("encode base"),
        )
        .expect("write base");
    cx_id
}

fn context(action_id: &str, row: &Row) -> Vec<u8> {
    let mut value = json!({
        "action": action_id,
        "oracle_verdict": { "value": { "text": row.outcome } },
        "outcome_anchor": { "value": { "text": row.outcome } }
    });
    if let Some(truth) = row.truth {
        value["ground_truth_anchor"] = json!({ "value": { "text": truth } });
    }
    serde_json::to_vec(&value).expect("context json")
}

fn put_sufficiency(
    vault: &AsterVault<SystemClock>,
    domain: &str,
    panel: &Panel,
    panel_bits: f32,
    entropy_bits: f32,
    slot_bits: &[f32],
) {
    let key = AssayCacheKey::scoped(panel.version, domain, vault.vault_id(), AnchorKind::Reward);
    let mut store = AssayStore::default();
    store.put(
        key.clone(),
        AssaySubject::Panel,
        estimate(panel_bits, EstimatorKind::PanelSufficiency),
        "ph49 fsv panel bits",
        calyx_testkit::DEFAULT_TEST_TS,
    );
    store.put(
        key.clone(),
        AssaySubject::OutcomeEntropy,
        estimate(entropy_bits, EstimatorKind::OutcomeEntropy),
        "ph49 fsv outcome entropy",
        calyx_testkit::DEFAULT_TEST_TS,
    );
    for (slot, bits) in panel.slots.iter().zip(slot_bits.iter().copied()) {
        store.put(
            key.clone(),
            AssaySubject::Lens { slot: slot.slot_id },
            estimate(bits, EstimatorKind::Ksg),
            "ph49 fsv lens bits",
            calyx_testkit::DEFAULT_TEST_TS,
        );
    }
    store.persist_to_vault(vault).expect("persist assay rows");
}

fn estimate(bits: f32, estimator: EstimatorKind) -> MiEstimate {
    MiEstimate::point(bits, 120, estimator, TrustTag::Trusted)
}

fn action(action_id: &str, panel: Panel) -> Action {
    Action {
        action_id: action_id.to_string(),
        panel,
        guard: None,
    }
}

fn panel(version: u32, slots: &[u16]) -> Panel {
    Panel {
        version,
        slots: slots.iter().copied().map(slot).collect(),
        created_at: calyx_testkit::DEFAULT_TEST_TS,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn slot(id: u16) -> Slot {
    let slot_id = SlotId::new(id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, format!("slot-{id}")),
        lens_id: LensId::from_bytes([id as u8; 16]),
        shape: SlotShape::Dense(2),
        modality: Modality::Code,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some("ph49-oracle-fsv".to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: EXEC_VERSION,
    }
}

fn constellation(
    vault_id: VaultId,
    cx_id: CxId,
    domain: &str,
    action_id: &str,
) -> calyx_core::Constellation {
    let mut metadata = BTreeMap::new();
    metadata.insert(ORACLE_DOMAIN_METADATA_KEY.to_string(), domain.to_string());
    metadata.insert(
        ORACLE_ACTION_METADATA_KEY.to_string(),
        action_id.to_string(),
    );
    calyx_core::Constellation {
        cx_id,
        vault_id,
        panel_version: EXEC_VERSION,
        created_at: calyx_testkit::DEFAULT_TEST_TS,
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

fn durable_vault(path: &Path) -> AsterVault<SystemClock> {
    let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path).expect("create vault dir");
    AsterVault::new_durable(
        path,
        vault_id(),
        b"issue434-fsv".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

fn prepare_fsv_root() -> PathBuf {
    let root = std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("calyx-issue434-{}", std::process::id()))
        });
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create fsv root");
    root
}

fn write_summary(root: &Path) {
    let summary = json!({
        "issue": 434,
        "domain": DOMAIN,
        "vault": root.join("vault"),
        "corrupt_vault": root.join("corrupt-vault"),
        "ceiling_jsonl": ceiling_log_path(),
    });
    let mut file = File::create(root.join("ph49_fsv_summary.json")).expect("summary");
    writeln!(file, "{}", serde_json::to_string_pretty(&summary).unwrap()).expect("summary bytes");
    print_json(json!({"fsv": "ph49_summary", "summary": summary}));
}

fn ceiling_log_path() -> PathBuf {
    if cfg!(windows) {
        std::env::temp_dir().join("calyx_oracle_ceiling_check.jsonl")
    } else {
        PathBuf::from("/tmp/calyx_oracle_ceiling_check.jsonl")
    }
}

fn print_json(value: serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string(&value).expect("serialize fsv evidence")
    );
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
