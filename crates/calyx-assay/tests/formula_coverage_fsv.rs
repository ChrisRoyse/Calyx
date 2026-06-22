use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_anneal::{
    ActionMetricSnapshot, AnnealAction, AnnealLedger, AnnealLedgerAction, AnnealSubstrate,
    ArtifactKey, ArtifactPtr, AsterAnnealLedgerStore, AsterRollbackStorage, BudgetConfig,
    BudgetEnforcer, BudgetProbe, BudgetProbeSample, ChangeOutcome, HeldOutReplay, ReplayAnchor,
    ReplayQuery, RollbackStore, TripwireMetric, TripwireRegistry, decode_anneal_ledger_payload,
};
use calyx_assay::{
    FORMULA_COVERAGE_SOT_KEY, FormulaCoverageArtifact, dpi_ceiling, formula_coverage_artifact,
    lens_signal, marginal_value, pair_redundancy, validate_formula_coverage,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, FixedClock, SlotId, VaultId};
use calyx_ledger::{ActorId, LedgerAppender, decode};
use calyx_loom::{dda_signal_yield, meaning_compression_yield};
use calyx_oracle::oracle_formula_predict;
use calyx_sextant::fusion::rrf::rrf_contribution;
use calyx_ward::{GuardId, GuardPolicy, GuardProfile, NoveltyAction, guard};
use serde::Serialize;
use serde_json::{Value, json};

const TEST_TS: u64 = 1_785_400_639;

#[test]
#[ignore = "manual FSV for #639 PRD-22 formula coverage"]
fn prd22_formula_coverage_persists_and_reads_back_from_aster() {
    let root = fsv_root();
    let vault_dir = root.join("vault");
    fs::create_dir_all(&vault_dir).expect("create fsv vault root");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue-639-formula-coverage".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");

    let assay_before = cf_rows(&vault, ColumnFamily::Assay);
    assert!(assay_before.is_empty());

    let artifact = formula_coverage_artifact(root.display().to_string(), TEST_TS);
    let summary = validate_formula_coverage(&artifact).expect("coverage summary");
    assert_eq!(summary.missing_rows, 0);
    let coverage_bytes = serde_json::to_vec_pretty(&artifact).expect("coverage json");
    let coverage_file = root.join("formula-coverage.json");
    fs::write(&coverage_file, &coverage_bytes).expect("write coverage artifact");

    let coverage_key = FORMULA_COVERAGE_SOT_KEY.as_bytes().to_vec();
    vault
        .write_cf(
            ColumnFamily::Assay,
            coverage_key.clone(),
            coverage_bytes.clone(),
        )
        .expect("write coverage assay cf");
    vault.flush().expect("flush coverage vault");

    let assay_after = cf_rows(&vault, ColumnFamily::Assay);
    let stored_bytes = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Assay, &coverage_key)
        .expect("read coverage row")
        .expect("coverage row exists");
    assert_eq!(stored_bytes, coverage_bytes);
    let stored_artifact: FormulaCoverageArtifact =
        serde_json::from_slice(&stored_bytes).expect("read artifact json from CF");
    assert_eq!(
        validate_formula_coverage(&stored_artifact).unwrap(),
        summary
    );

    let spot_checks = spot_checks();
    let edge_cases = edge_cases(&artifact);
    let self_tuning = self_tuning_readback(&vault, &vault_dir);
    vault.flush().expect("flush anneal vault");

    let ledger_after = cf_rows(&vault, ColumnFamily::Ledger);
    let rollback_after = cf_rows(&vault, ColumnFamily::AnnealRollback);
    let readback = json!({
        "source_of_truth": {
            "coverage": format!("Aster assay CF key {}", FORMULA_COVERAGE_SOT_KEY),
            "anneal_ledger": "Aster ledger CF",
            "anneal_rollback": "Aster anneal_rollback CF",
            "wal": vault_dir.join("wal").display().to_string(),
            "artifact_file": coverage_file.display().to_string()
        },
        "coverage_summary": summary,
        "assay_cf_before": assay_before,
        "assay_cf_after": assay_after,
        "spot_checks": spot_checks,
        "edge_cases": edge_cases,
        "self_tuning": self_tuning,
        "ledger_cf_after": ledger_after,
        "rollback_cf_after": rollback_after,
        "physical_files": physical_files(&vault_dir),
    });
    let readback_file = root.join("formula-coverage-readback.json");
    write_json(&readback_file, &readback);
    write_checksums(&root, &[&coverage_file, &readback_file]);

    println!("ISSUE639_FORMULA_FSV {}", root.display());
}

fn spot_checks() -> Value {
    let dda = dda_signal_yield(10, 13);
    let rrf = rrf_contribution(1.0, 1) + rrf_contribution(2.0, 4);
    let expected_rrf = 1.0 / 61.0 + 2.0 / 64.0;
    let ward = gtau_verdict();
    assert_eq!(dda, 920);
    assert!((rrf - expected_rrf).abs() <= 1.0e-7);
    assert!(ward["overall_pass"].as_bool().unwrap());

    json!({
        "dda_signals_10_by_13": { "expected": 920, "actual": dda },
        "rrf_score": { "expected": expected_rrf, "actual": rrf },
        "gtau_verdict": ward,
        "lens_signal": lens_signal(0.05, 0.6).expect("lens signal pass"),
        "meaning_compression": meaning_compression_yield(920, 10),
        "marginal_value": marginal_value(1.25, 0.75).expect("marginal value"),
        "dpi_ceiling": dpi_ceiling(2.0).expect("dpi ceiling")
    })
}

fn gtau_verdict() -> Value {
    let slot = SlotId::new(8);
    let mut tau = BTreeMap::new();
    tau.insert(slot, 0.7);
    let profile = GuardProfile {
        guard_id: "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101"
            .parse::<GuardId>()
            .expect("guard id"),
        panel_version: 639,
        domain: "issue639".to_string(),
        tau,
        required_slots: vec![slot],
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    };
    let produced = BTreeMap::from([(slot, vec![1.0, 0.0])]);
    let matched = BTreeMap::from([(slot, vec![0.8, 0.6])]);
    let verdict = guard(&profile, &produced, &matched, false).expect("gtau guard");
    assert!((verdict.per_slot[0].cos - 0.8).abs() <= 1.0e-6);
    serde_json::to_value(verdict).expect("gtau json")
}

fn edge_cases(artifact: &FormulaCoverageArtifact) -> Value {
    let mut empty_artifact = artifact.clone();
    empty_artifact.rows.clear();
    empty_artifact.summary.total_rows = 0;
    empty_artifact.summary.covered_rows = 0;
    empty_artifact.summary.missing_rows = 0;
    let empty_error = validate_formula_coverage(&empty_artifact).unwrap_err();
    let redundant_error = pair_redundancy(0.61).unwrap_err();
    let oracle_error = oracle_formula_predict(0.25, 0.75, 0.8).unwrap_err();

    json!({
        "empty_catalog": {
            "before": { "rows": artifact.rows.len(), "missing": artifact.summary.missing_rows },
            "after": error_json(&empty_error)
        },
        "pair_redundancy_over_threshold": {
            "before": { "correlation": 0.61, "max_allowed": 0.60 },
            "after": error_json(&redundant_error)
        },
        "oracle_insufficient_panel_bits": {
            "before": { "panel_bits": 0.25, "anchor_entropy_bits": 0.75 },
            "after": error_json(&oracle_error)
        }
    })
}

fn self_tuning_readback(vault: &AsterVault, vault_dir: &Path) -> Value {
    let clock = FixedClock::new(TEST_TS);
    let tripwires = TripwireRegistry::load_from_vault(vault_dir).expect("tripwires");
    let replay = HeldOutReplay {
        queries: vec![ReplayQuery {
            query_id: 639,
            query_vector: vec![1.0],
            expected_top_k: vec![ReplayAnchor {
                cx_id: cx(0x91),
                similarity: 1.0,
            }],
        }],
        seed: 639,
    };
    let rollback =
        RollbackStore::open(&clock, 639, AsterRollbackStorage::new(vault)).expect("rollback store");
    let ledger = AnnealLedger::new(
        LedgerAppender::open(AsterAnnealLedgerStore::new(vault), clock).expect("ledger appender"),
        ActorId::Service("formula-coverage-fsv".to_string()),
    )
    .expect("anneal ledger");
    let budget = BudgetEnforcer::with_probe(
        BudgetConfig {
            cpu_fraction: 1.0,
            vram_bytes: 1,
            tick_interval_ms: 1,
        },
        &clock,
        ScriptedProbe,
    )
    .expect("budget enforcer");
    let mut substrate = AnnealSubstrate::new(tripwires, replay, rollback, ledger, budget, &clock);

    let rrf_key = ArtifactKey::ConfigCache([0xA1; 32]);
    let ksg_key = ArtifactKey::ConfigCache([0xA2; 32]);
    substrate
        .rollback
        .install_live_ptr(rrf_key.clone(), ArtifactPtr::ConfigCacheKeyHash([0xB1; 32]))
        .expect("install rrf live ptr");
    substrate
        .rollback
        .install_live_ptr(ksg_key.clone(), ArtifactPtr::ConfigCacheKeyHash([0xB2; 32]))
        .expect("install ksg live ptr");

    let incumbent = ParamAction { recall: 0.92 };
    let candidate = ParamAction { recall: 0.95 };
    let rrf = substrate
        .propose_change_with_description(
            rrf_key,
            ArtifactPtr::ConfigCacheKeyHash([0xC1; 32]),
            &candidate,
            &incumbent,
            "rrf.k self-tuning representative",
        )
        .expect("rrf promotion");
    let ksg = substrate
        .propose_change_with_description(
            ksg_key,
            ArtifactPtr::ConfigCacheKeyHash([0xC2; 32]),
            &candidate,
            &incumbent,
            "ksg.k self-tuning representative",
        )
        .expect("ksg promotion");
    assert!(matches!(rrf, ChangeOutcome::Promoted(_)));
    assert!(matches!(ksg, ChangeOutcome::Promoted(_)));

    let ledger_entries = decode_anneal_rows(vault);
    let rollback_rows = cf_rows(vault, ColumnFamily::AnnealRollback);
    assert_eq!(ledger_entries.len(), 2);
    assert!(rollback_rows.len() >= 4);
    json!({
        "outcomes": { "rrf": rrf, "ksg": ksg },
        "ledger_entries": ledger_entries,
        "rollback_row_count": rollback_rows.len(),
        "rollback_rows": rollback_rows
    })
}

#[derive(Clone, Copy)]
struct ScriptedProbe;

impl BudgetProbe for ScriptedProbe {
    fn sample(&self) -> BudgetProbeSample {
        BudgetProbeSample {
            cpu_used_fraction: 0.0,
            vram_used_bytes: 0,
            nvml_available: true,
        }
    }
}

struct ParamAction {
    recall: f64,
}

impl AnnealAction for ParamAction {
    fn apply_shadow(&self, _query: &ReplayQuery) -> ActionMetricSnapshot {
        ActionMetricSnapshot::from_values([
            (TripwireMetric::RecallAtK, self.recall),
            (TripwireMetric::GuardFAR, 0.004),
            (TripwireMetric::GuardFRR, 0.015),
            (TripwireMetric::SearchP99, 40.0),
            (TripwireMetric::IngestP95, 70.0),
        ])
    }
}

fn decode_anneal_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .expect("scan ledger")
        .into_iter()
        .map(|(key, value)| {
            let entry = decode(&value).expect("decode ledger entry");
            let payload = decode_anneal_ledger_payload(&entry.payload).expect("decode anneal");
            assert_eq!(payload.action, AnnealLedgerAction::Promote);
            json!({
                "key_hex": hex(&key),
                "seq": entry.seq,
                "entry_hash": hex(&entry.entry_hash),
                "action": payload.action,
                "description": payload.description,
                "change_id": payload.change_id.0,
                "metrics": payload.metrics
            })
        })
        .collect()
}

fn cf_rows(vault: &AsterVault, cf: ColumnFamily) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), cf)
        .expect("scan cf")
        .into_iter()
        .map(|(key, value)| {
            json!({
                "cf": cf.name(),
                "key_hex": hex(&key),
                "key_utf8": String::from_utf8(key.clone()).ok(),
                "value_len": value.len(),
                "value_blake3": blake3::hash(&value).to_hex().to_string(),
                "value_utf8_prefix": utf8_prefix(&value),
            })
        })
        .collect()
}

fn physical_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_files(root, &mut files);
    files.sort();
    files
}

fn collect_files(root: &Path, out: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files(&path, out);
            } else {
                out.push(path.display().to_string());
            }
        }
    }
}

fn write_json(path: &Path, value: &impl Serialize) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize readback json"),
    )
    .expect("write readback json");
}

fn write_checksums(root: &Path, files: &[&PathBuf]) {
    let mut lines = Vec::new();
    for path in files {
        let bytes = fs::read(path).expect("checksum read");
        lines.push(format!(
            "{}  {}",
            blake3::hash(&bytes).to_hex(),
            path.display()
        ));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), lines.join("\n")).expect("write checksums");
}

fn error_json(error: &calyx_core::CalyxError) -> Value {
    json!({ "code": error.code, "message": error.message, "remediation": error.remediation })
}

fn utf8_prefix(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    Some(text.chars().take(240).collect())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn fsv_root() -> PathBuf {
    let base = std::env::var("CALYX_ISSUE639_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue639-formula-coverage-fsv"));
    let root = base.join(format!("issue639-{}", std::process::id()));
    fs::create_dir_all(&root).expect("create fsv root");
    root
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}
