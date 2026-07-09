use super::*;

fn chain() -> Arc<ChainVerifyMetrics> {
    Arc::new(ChainVerifyMetrics::new(&["/data/vault-a".to_string()]))
}

fn metrics() -> CalyxMetrics {
    CalyxMetrics::new(chain(), &["/data/vault-a".to_string()])
}

#[test]
fn new_registers_at_least_30_preinitialized_families() {
    let metrics = metrics();
    // 24 T03 families (2 ingest, 3 search, 2 guard*, 2 assay/kernel*,
    // 2 anneal*, 6 vram, 7 verify-restore) + 25 hazards. The
    // guard/assay/kernel/anneal/VRAM-audit Vec
    // families have no series until observed, so the live count after
    // pre-init is the always-present ones: 2 vram + 7 restore + 25 hazard
    // + 5 vault-seeded ingest/search = 39.
    assert!(
        metrics.family_count() >= 39,
        "expected >= 39 families, got {}",
        metrics.family_count()
    );
    let text = metrics.encode_text().unwrap();
    // Chain-verify family is composed in.
    assert!(text.contains("calyx_ledger_chain_verify_ok"));
    // Recall tripwire pre-initialized healthy.
    assert!(text.contains("calyx_search_recall_tripwire{vault=\"/data/vault-a\"} 1"));
}

#[test]
fn ingest_observation_increments_counter_and_histogram() {
    let metrics = metrics();
    metrics.observe_ingest("/data/vault-a", 0.150, true);
    let text = metrics.encode_text().unwrap();
    assert!(text.contains("calyx_ingest_total{status=\"ok\",vault=\"/data/vault-a\"} 1"));
    assert!(text.contains("calyx_ingest_duration_seconds_count{vault=\"/data/vault-a\"} 1"));
    // 0.150s falls in the le="0.25" bucket but not le="0.1".
    assert!(
        text.contains(
            "calyx_ingest_duration_seconds_bucket{vault=\"/data/vault-a\",le=\"0.25\"} 1"
        )
    );
    assert!(
        text.contains("calyx_ingest_duration_seconds_bucket{vault=\"/data/vault-a\",le=\"0.1\"} 0")
    );
}

#[test]
fn recall_tripwire_tripped_emits_zero() {
    let metrics = metrics();
    metrics.set_recall_tripwire("/data/vault-a", false);
    let text = metrics.encode_text().unwrap();
    assert!(text.contains("calyx_search_recall_tripwire{vault=\"/data/vault-a\"} 0"));
}

#[test]
fn vram_budget_exact_text_match() {
    let metrics = metrics();
    metrics.set_vram_budget(4096, 8192);
    let text = metrics.encode_text().unwrap();
    assert!(text.contains("calyx_vram_budget_used_mib 4096"));
    assert!(text.contains("calyx_vram_budget_limit_mib 8192"));
}

#[test]
fn vram_audit_records_labeled_nvml_readback() {
    let metrics = metrics();
    metrics.record_vram_budget_audit(
        "/data/vault-a",
        "runtime",
        &VramAuditReport {
            tei_used_mib: 4096,
            calyx_budget_mib: 8192,
            device_total_mib: 32607,
        },
    );
    let text = metrics.encode_text().unwrap();
    assert!(
        text.contains(
            "calyx_vram_budget_audit_resident_mib{panel=\"runtime\",vault=\"/data/vault-a\"} 4096"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "calyx_vram_budget_audit_budget_mib{panel=\"runtime\",vault=\"/data/vault-a\"} 8192"
        ),
        "{text}"
    );
    assert!(
            text.contains(
                "calyx_vram_budget_audit_device_total_mib{panel=\"runtime\",vault=\"/data/vault-a\"} 32607"
            ),
            "{text}"
        );
    assert!(
        text.contains(
            "calyx_vram_budget_audit_headroom_mib{panel=\"runtime\",vault=\"/data/vault-a\"} 20319"
        ),
        "{text}"
    );
}

#[test]
fn verify_restore_records_pass_fail_gauges_and_counts() {
    let metrics = metrics();
    let report = VerifyRestoreReport {
        vault_path: "/data/vault-a".into(),
        constellation_count: 3,
        anchor_count: 5,
        ledger_entry_count: 7,
        ledger_tip_hash: "abc123".to_string(),
        chain_intact: true,
        wal_bytes_present: 2048,
        first_cx_id: Some("001122".to_string()),
        error: None,
    };
    metrics.record_verify_restore("/data/vault-a", &report, 1_770_000_123);
    let text = metrics.encode_text().unwrap();
    assert!(text.contains("calyx_verify_restore_ok{vault=\"/data/vault-a\"} 1"));
    assert!(text.contains("calyx_verify_restore_chain_intact{vault=\"/data/vault-a\"} 1"));
    assert!(text.contains(
        "calyx_verify_restore_last_run_timestamp_seconds{vault=\"/data/vault-a\"} 1770000123"
    ));
    assert!(text.contains("calyx_verify_restore_constellation_count{vault=\"/data/vault-a\"} 3"));
    assert!(text.contains("calyx_verify_restore_anchor_count{vault=\"/data/vault-a\"} 5"));
    assert!(text.contains("calyx_verify_restore_ledger_entry_count{vault=\"/data/vault-a\"} 7"));
    assert!(text.contains("calyx_verify_restore_wal_bytes_present{vault=\"/data/vault-a\"} 2048"));
}

#[test]
fn search_strategy_and_guard_families_appear_on_record() {
    let metrics = metrics();
    metrics.observe_search("/data/vault-a", SearchStrategy::WeightedRrf, 0.02, true);
    metrics.set_guard_rates("/data/vault-a", "subject", 0.01, 0.02);
    metrics.set_assay_n_eff("/data/vault-a", "default", 128.0);
    let text = metrics.encode_text().unwrap();
    assert!(text.contains(
        "calyx_search_total{status=\"ok\",strategy=\"weighted_rrf\",vault=\"/data/vault-a\"} 1"
    ));
    assert!(text.contains("calyx_guard_far{slot=\"subject\",vault=\"/data/vault-a\"} 0.01"));
    assert!(text.contains("calyx_guard_frr{slot=\"subject\",vault=\"/data/vault-a\"} 0.02"));
    assert!(text.contains("calyx_assay_n_eff{panel=\"default\",vault=\"/data/vault-a\"} 128"));
}

#[test]
fn all_25_hazards_present_and_zero_at_init() {
    let metrics = metrics();
    let text = metrics.encode_text().unwrap();
    let hazard_lines: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("calyx_hazard_"))
        .collect();
    assert_eq!(hazard_lines.len(), 25, "expected 25 hazard value lines");
    for line in &hazard_lines {
        assert!(line.ends_with(" 0"), "hazard not zero at init: {line}");
    }
}

#[test]
fn set_hazard_unknown_is_fail_closed() {
    let metrics = metrics();
    assert!(metrics.set_hazard("nope", true).is_err());
    metrics.set_hazard("disk_full", true).unwrap();
    let text = metrics.encode_text().unwrap();
    assert!(text.contains("calyx_hazard_disk_full{hazard=\"disk_full\"} 1"));
}
